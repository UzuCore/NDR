//! Blocking remote services. Call these APIs from a worker, never the UI thread.
//! Passwords are runtime-only; this module deliberately does not implement serde.

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use sha2::{Digest, Sha256};
use ssh2::{OpenFlags, OpenType, Session, Sftp};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use suppaftp::native_tls;
use suppaftp::{
    NativeTlsConnector, NativeTlsFtpStream, Status,
    types::{FileType, FtpError, FtpResult, Response},
};

const TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Control connection lines received by the original pane FTP log box (TListBox, FUN_0048d7ac) (original Indy TIdLogEvent -> FUN_004404c0).
/// Received lines as is (trailing "<EOL>" removed), sent lines as "current remote folder > command" (0x8ed8a6 "%s > %s"), and
/// with "PASS " present, masked with '*' from the sixth character. Each added line selects the last line (ItemIndex = Count - 1).
#[derive(Debug, Default)]
pub struct ProtocolLog {
    pub lines: Vec<String>,
    /// Prefix of sent lines (original FTP engine +0x1c: current remote folder).
    pub prefix: String,
    pub selected: Option<usize>,
    /// suppaftp logs the first line of a multi-line reply twice, so the next received line is skipped.
    skip_received: bool,
}

impl ProtocolLog {
    pub fn sent(&mut self, command: &str) {
        let command = command.trim_end_matches(['\r', '\n']);
        let masked: String = if command.contains("PASS ") {
            command.chars().enumerate().map(|(i, c)| if i >= 5 { '*' } else { c }).collect()
        } else {
            command.to_owned()
        };
        let line = format!("{} > {masked}", self.prefix);
        self.push(line);
    }
    pub fn received(&mut self, line: &str) {
        self.push(line.trim_end_matches(['\r', '\n']).to_owned());
    }
    fn push(&mut self, line: String) {
        self.lines.push(line);
        self.selected = Some(self.lines.len() - 1);
    }
}

thread_local! {
    static PROTOCOL_LOG: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<ProtocolLog>>>> = const { std::cell::RefCell::new(None) };
}

/// Records the FTP control lines `f` sends and receives on this thread into `log` (nothing with None).
pub fn with_protocol_log<R>(log: Option<&std::sync::Arc<std::sync::Mutex<ProtocolLog>>>, f: impl FnOnce() -> R) -> R {
    install_protocol_logger();
    let previous = PROTOCOL_LOG.with(|slot| slot.replace(log.cloned()));
    let result = f();
    PROTOCOL_LOG.with(|slot| *slot.borrow_mut() = previous);
    result
}

fn protocol_log(f: impl FnOnce(&mut ProtocolLog)) {
    PROTOCOL_LOG.with(|slot| {
        if let Some(log) = slot.borrow().as_ref() {
            if let Ok(mut log) = log.lock() {
                f(&mut log);
            }
        }
    });
}

/// suppaftp logs control lines through the log crate ("CC OUT: command", "CC IN: [bytes]"). They are routed to the log box.
struct SuppaftpLog;
impl log::Log for SuppaftpLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().starts_with("suppaftp")
    }
    fn log(&self, record: &log::Record) {
        if !record.target().starts_with("suppaftp") || PROTOCOL_LOG.with(|slot| slot.borrow().is_none()) {
            return;
        }
        let message = record.args().to_string();
        if let Some(command) = message.strip_prefix("CC OUT: ") {
            protocol_log(|log| log.sent(command));
        } else if let Some(bytes) = message.strip_prefix("CC IN: ") {
            let bytes: Vec<u8> = bytes.trim_matches(['[', ']']).split(',').filter_map(|b| b.trim().parse().ok()).collect();
            let text = String::from_utf8_lossy(&bytes);
            protocol_log(|log| {
                if !std::mem::take(&mut log.skip_received) {
                    log.received(&text);
                }
            });
        } else if message.starts_with("Code parsed from response") {
            protocol_log(|log| log.skip_received = true);
        }
    }
    fn flush(&self) {}
}

fn install_protocol_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if log::set_logger(&SuppaftpLog).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    });
}

#[derive(Clone, Default)]
pub struct Config {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub private_key: PathBuf,
    pub key_passphrase: String,
    pub fingerprint: String,
    pub passive: bool,
    pub encoding_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub is_symlink: bool,
    /// Last modification time (Unix seconds, UTC). None when the server does not report it.
    pub modified: Option<i64>,
    /// Permission bits (within 0o7777). None when the server does not report them (C# RemoteEntry.PermissionMode).
    pub mode: Option<u32>,
    /// The listing's own permission text: the MLSD "perm" fact ("eldfmcp", "radfw") or the LIST mode column ("drwxr-xr-x").
    /// The original FTP list shows it in the attribute cell (Synapse TFTPListRec.Permission, item +0x74). None for SFTP and DOS listings.
    pub permission: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TransferProgress {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
}

/// Information passed when a tree transfer asks about a name conflict. Original FUN_00467764 (filling TfFileExists): shows the remote size and time,
/// and btnResume is on for FTP when the server accepts REST (Synapse CanResume) and always for SFTP.
pub struct ConflictInfo<'a> {
    /// The remote item (download = source, upload = destination).
    pub remote: &'a RemoteEntry,
    /// Whether the local side is a folder.
    pub local_dir: bool,
    /// True for downloads (remote = source), false for uploads (remote = destination).
    pub remote_is_source: bool,
    pub can_resume: bool,
    /// New name the asking side fills in for "rename" (original FUN_00467658 input window). When empty, a free name like " (1)" is chosen.
    pub new_name: std::cell::RefCell<Option<String>>,
}

/// Original FUN_00467dfc and FUN_00469210: "overwrite if newer" overwrites when the new item's time is later than the existing one, otherwise skips.
/// Unknown times count as 0 (original FILETIME 0).
fn newer_wins(new: Option<i64>, existing: Option<i64>) -> crate::fsops::ConflictChoice {
    if new.unwrap_or(0) > existing.unwrap_or(0) {
        crate::fsops::ConflictChoice::Overwrite
    } else {
        crate::fsops::ConflictChoice::Skip
    }
}

fn local_seconds(meta: &fs::Metadata) -> Option<i64> {
    meta.modified().ok().map(|time| filetime::FileTime::from_system_time(time).unix_seconds())
}

trait TransferStream: Read + Write {
    fn finish_write(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}
struct SdkData<T>(T);
impl<T: Read> Read for SdkData<T> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl<T: Write> Write for SdkData<T> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl<T: Read + Write> TransferStream for SdkData<T> {}

struct Ftp {
    stream: FtpControl,
    passive: bool,
    tls: Option<native_tls::TlsConnector>,
    host: String,
    encoding: &'static encoding_rs::Encoding,
}

enum Wire {
    Plain(TcpStream),
    Secure(Box<native_tls::TlsStream<TcpStream>>),
}
impl Wire {
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Plain(s) => s,
            Self::Secure(s) => s.get_ref(),
        }
    }
}
impl Read for Wire {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(bytes),
            Self::Secure(s) => s.read(bytes),
        }
    }
}
impl Write for Wire {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(bytes),
            Self::Secure(s) => s.write(bytes),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Secure(s) => s.flush(),
        }
    }
}
impl TransferStream for Wire {
    fn finish_write(&mut self) -> std::io::Result<()> {
        self.flush()?;
        if let Self::Secure(stream) = self {
            stream.shutdown().map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

// suppaftp's public stream always emits UTF-8. Keep its tested default path;
// this small control transport supplies exact legacy codepage bytes only.
struct EncodedFtp {
    control: BufReader<Wire>,
    encoding: &'static encoding_rs::Encoding,
    tls: Option<native_tls::TlsConnector>,
    host: String,
}

impl EncodedFtp {
    fn connect(
        stream: TcpStream,
        config: &Config,
        encoding: &'static encoding_rs::Encoding,
        tls: Option<native_tls::TlsConnector>,
    ) -> Result<Self, String> {
        let mut ftp = Self {
            control: BufReader::new(Wire::Plain(stream)),
            encoding,
            tls: tls.clone(),
            host: config.host.clone(),
        };
        ftp.response(&[Status::Ready]).map_err(err)?;
        if let Some(connector) = tls {
            ftp.custom_command("AUTH TLS", &[Status::AuthOk])
                .map_err(err)?;
            let Wire::Plain(stream) = ftp.control.into_inner() else {
                unreachable!()
            };
            let secure = connector.connect(&config.host, stream).map_err(err)?;
            ftp.control = BufReader::new(Wire::Secure(Box::new(secure)));
            ftp.custom_command("PBSZ 0", &[Status::CommandOk])
                .map_err(err)?;
            ftp.custom_command("PROT P", &[Status::CommandOk])
                .map_err(err)?;
        }
        let user = ftp
            .custom_command(
                &format!("USER {}", config.user),
                &[Status::NeedPassword, Status::LoggedIn],
            )
            .map_err(err)?;
        if user.status == Status::NeedPassword {
            ftp.custom_command(&format!("PASS {}", config.password), &[Status::LoggedIn])
                .map_err(err)?;
        }
        ftp.custom_command("TYPE I", &[Status::CommandOk])
            .map_err(err)?;
        let _ = ftp.custom_command("OPTS UTF8 OFF", &[Status::CommandOk]);
        Ok(ftp)
    }

    fn response(&mut self, expected: &[Status]) -> FtpResult<Response> {
        let mut bytes = Vec::new();
        self.control
            .read_until(b'\n', &mut bytes)
            .map_err(FtpError::ConnectionError)?;
        if bytes.len() < 4 || !bytes[..3].iter().all(u8::is_ascii_digit) {
            return Err(FtpError::BadResponse);
        }
        protocol_log(|log| log.received(&self.encoding.decode_without_bom_handling(&bytes).0));
        let code = std::str::from_utf8(&bytes[..3])
            .map_err(|_| FtpError::BadResponse)?
            .parse::<u32>()
            .map_err(|_| FtpError::BadResponse)?;
        if bytes[3] == b'-' {
            let terminator = format!("{code:03} ");
            loop {
                let mut line = Vec::new();
                if self
                    .control
                    .read_until(b'\n', &mut line)
                    .map_err(FtpError::ConnectionError)?
                    == 0
                {
                    return Err(FtpError::BadResponse);
                }
                let finished = line.starts_with(terminator.as_bytes());
                protocol_log(|log| log.received(&self.encoding.decode_without_bom_handling(&line).0));
                bytes.extend_from_slice(&line);
                if bytes.len() > 1024 * 1024 {
                    return Err(FtpError::BadResponse);
                }
                if finished {
                    break;
                }
            }
        } else if bytes[3] != b' ' {
            return Err(FtpError::BadResponse);
        }
        let text = self
            .encoding
            .decode_without_bom_handling_and_without_replacement(&bytes)
            .ok_or(FtpError::BadResponse)?;
        let response = Response::new(Status::from(code), text.into_owned().into_bytes());
        if expected.contains(&response.status) {
            Ok(response)
        } else {
            Err(FtpError::UnexpectedResponse(response))
        }
    }

    fn custom_command(&mut self, command: &str, expected: &[Status]) -> FtpResult<Response> {
        if validate_text(command).is_err() {
            return Err(FtpError::BadResponse);
        }
        let (bytes, _, replacement) = self.encoding.encode(command);
        if replacement
            || self
                .encoding
                .decode_without_bom_handling_and_without_replacement(&bytes)
                .as_deref()
                != Some(command)
        {
            return Err(FtpError::ConnectionError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "FTP text cannot be represented exactly in the selected encoding",
            )));
        }
        protocol_log(|log| log.sent(command));
        self.control
            .get_mut()
            .write_all(&bytes)
            .map_err(FtpError::ConnectionError)?;
        self.control
            .get_mut()
            .write_all(b"\r\n")
            .map_err(FtpError::ConnectionError)?;
        self.control
            .get_mut()
            .flush()
            .map_err(FtpError::ConnectionError)?;
        self.response(expected)
    }
    fn pwd(&mut self) -> FtpResult<String> {
        let response = self
            .custom_command("PWD", &[Status::PathCreated])?
            .as_string()
            .map_err(|_| FtpError::BadResponse)?;
        let mut chars = response
            .split_once('"')
            .ok_or(FtpError::BadResponse)?
            .1
            .chars()
            .peekable();
        let mut path = String::new();
        while let Some(c) = chars.next() {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    path.push('"');
                } else {
                    return Ok(path);
                }
            } else {
                path.push(c);
            }
        }
        Err(FtpError::BadResponse)
    }
    fn cwd(&mut self, path: &str) -> FtpResult<()> {
        self.custom_command(&format!("CWD {path}"), &[Status::RequestedFileActionOk])
            .map(|_| ())
    }
    fn mkdir(&mut self, path: &str) -> FtpResult<()> {
        self.custom_command(
            &format!("MKD {path}"),
            &[Status::PathCreated, Status::RequestedFileActionOk],
        )
        .map(|_| ())
    }
    fn rmdir(&mut self, path: &str) -> FtpResult<()> {
        self.custom_command(&format!("RMD {path}"), &[Status::RequestedFileActionOk])
            .map(|_| ())
    }
    fn rm(&mut self, path: &str) -> FtpResult<()> {
        self.custom_command(&format!("DELE {path}"), &[Status::RequestedFileActionOk])
            .map(|_| ())
    }
    fn quit(&mut self) -> FtpResult<()> {
        self.custom_command("QUIT", &[Status::Closing]).map(|_| ())
    }
    fn rename(&mut self, source: &str, destination: &str) -> FtpResult<()> {
        self.custom_command(&format!("RNFR {source}"), &[Status::RequestFilePending])?;
        self.custom_command(
            &format!("RNTO {destination}"),
            &[Status::RequestedFileActionOk],
        )
        .map(|_| ())
    }
    fn size(&mut self, path: &str) -> FtpResult<usize> {
        let response = self
            .custom_command(&format!("SIZE {path}"), &[Status::File])?
            .as_string()
            .map_err(|_| FtpError::BadResponse)?;
        response[3..]
            .trim()
            .parse()
            .map_err(|_| FtpError::BadResponse)
    }
    fn mlst(&mut self, path: Option<&str>) -> FtpResult<String> {
        self.custom_command(
            &format!("MLST {}", path.unwrap_or("")),
            &[Status::RequestedFileActionOk],
        )?
        .as_string()
        .map_err(|_| FtpError::BadResponse)
    }
    fn site(&mut self, command: String) -> FtpResult<Response> {
        self.custom_command(&format!("SITE {command}"), &[Status::CommandOk])
    }
    fn eprt(&mut self, address: SocketAddr) -> FtpResult<()> {
        self.custom_command(
            &format!(
                "EPRT |{}|{}|{}|",
                if address.is_ipv4() { 1 } else { 2 },
                address.ip(),
                address.port()
            ),
            &[Status::CommandOk],
        )
        .map(|_| ())
    }
    fn passive_data(&mut self, command: &str) -> FtpResult<Wire> {
        let mut address = self
            .control
            .get_ref()
            .tcp()
            .peer_addr()
            .map_err(FtpError::ConnectionError)?;
        let port = if address.is_ipv6() {
            let response = self
                .custom_command("EPSV", &[Status::ExtendedPassiveMode])?
                .as_string()
                .map_err(|_| FtpError::BadResponse)?;
            let inner = response
                .split_once('(')
                .and_then(|p| p.1.split_once(')'))
                .ok_or(FtpError::BadResponse)?
                .0;
            inner
                .split('|')
                .nth(3)
                .ok_or(FtpError::BadResponse)?
                .parse()
                .map_err(|_| FtpError::BadResponse)?
        } else {
            let response = self
                .custom_command("PASV", &[Status::PassiveMode])?
                .as_string()
                .map_err(|_| FtpError::BadResponse)?;
            let inner = response
                .split_once('(')
                .and_then(|p| p.1.split_once(')'))
                .ok_or(FtpError::BadResponse)?
                .0;
            let parts = inner
                .split(',')
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| FtpError::BadResponse)?;
            if parts.len() != 6 {
                return Err(FtpError::BadResponse);
            }
            u16::from(parts[4]) * 256 + u16::from(parts[5])
        };
        address.set_port(port);
        let data =
            TcpStream::connect_timeout(&address, TIMEOUT).map_err(FtpError::ConnectionError)?;
        set_timeout(&data).map_err(|e| FtpError::ConnectionError(std::io::Error::other(e)))?;
        self.custom_command(command, &[Status::AboutToSend, Status::AlreadyOpen])?;
        match &self.tls {
            Some(connector) => connector
                .connect(&self.host, data)
                .map(|stream| Wire::Secure(Box::new(stream)))
                .map_err(|e| FtpError::SecureError(e.to_string())),
            None => Ok(Wire::Plain(data)),
        }
    }
    fn listing(&mut self, command: &str) -> FtpResult<Vec<String>> {
        let mut data = self.passive_data(command)?;
        let mut bytes = Vec::new();
        data.read_to_end(&mut bytes)
            .map_err(FtpError::ConnectionError)?;
        drop(data);
        self.response(&[Status::ClosingDataConnection, Status::RequestedFileActionOk])?;
        let text = self
            .encoding
            .decode_without_bom_handling_and_without_replacement(&bytes)
            .ok_or(FtpError::BadResponse)?;
        Ok(text
            .split_terminator('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
            .filter(|line| !line.is_empty())
            .collect())
    }
    fn list(&mut self, path: Option<&str>) -> FtpResult<Vec<String>> {
        self.listing(&format!(
            "LIST{}",
            path.map(|p| format!(" {p}")).unwrap_or_default()
        ))
    }
    fn mlsd(&mut self, path: Option<&str>) -> FtpResult<Vec<String>> {
        self.listing(&format!(
            "MLSD{}",
            path.map(|p| format!(" {p}")).unwrap_or_default()
        ))
    }
}

enum FtpControl {
    Utf8(NativeTlsFtpStream),
    Encoded(EncodedFtp),
}
macro_rules! ftp_dispatch {
    ($(fn $name:ident($($arg:ident: $kind:ty),*) -> $result:ty;)*) => {
        $(fn $name(&mut self, $($arg: $kind),*) -> $result {
            match self { Self::Utf8(ftp) => ftp.$name($($arg),*), Self::Encoded(ftp) => ftp.$name($($arg),*) }
        })*
    }
}
impl FtpControl {
    ftp_dispatch! {
        fn pwd() -> FtpResult<String>;
        fn cwd(path: &str) -> FtpResult<()>;
        fn mkdir(path: &str) -> FtpResult<()>;
        fn rmdir(path: &str) -> FtpResult<()>;
        fn rm(path: &str) -> FtpResult<()>;
        fn quit() -> FtpResult<()>;
        fn rename(source: &str, destination: &str) -> FtpResult<()>;
        fn size(path: &str) -> FtpResult<usize>;
        fn mlst(path: Option<&str>) -> FtpResult<String>;
        fn site(command: String) -> FtpResult<Response>;
        fn eprt(address: SocketAddr) -> FtpResult<()>;
        fn list(path: Option<&str>) -> FtpResult<Vec<String>>;
        fn mlsd(path: Option<&str>) -> FtpResult<Vec<String>>;
        fn custom_command(command: &str, expected: &[Status]) -> FtpResult<Response>;
    }
    fn get_ref(&self) -> &TcpStream {
        match self {
            Self::Utf8(ftp) => ftp.get_ref(),
            Self::Encoded(ftp) => ftp.control.get_ref().tcp(),
        }
    }
    /// REST n (350): the next RETR/STOR starts at byte n. Same as Synapse TFTPSend.InternalRetr/InternalStor (RestoreAt > 0).
    fn rest(&mut self, offset: u64) -> FtpResult<()> {
        self.custom_command(&format!("REST {offset}"), &[Status::RequestFilePending]).map(|_| ())
    }
    fn mtime(&mut self, path: &str) -> FtpResult<i64> {
        let response = self
            .custom_command(&format!("MDTM {path}"), &[Status::File])?
            .as_string()
            .map_err(|_| FtpError::BadResponse)?;
        let timestamp = response.get(3..).ok_or(FtpError::BadResponse)?.trim();
        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y%m%d%H%M%S")
            .map(|time| time.and_utc().timestamp())
            .map_err(|_| FtpError::BadResponse)
    }
    fn set_mtime(&mut self, path: &str, seconds: i64) -> FtpResult<()> {
        let time = chrono::DateTime::from_timestamp(seconds, 0).ok_or(FtpError::BadResponse)?;
        self.custom_command(
            &format!("MFMT {} {path}", time.format("%Y%m%d%H%M%S")),
            &[
                Status::File,
                Status::CommandOk,
                Status::RequestedFileActionOk,
            ],
        )
        .map(|_| ())
    }
    fn data(&mut self, verb: &str, path: &str) -> Result<Box<dyn TransferStream>, String> {
        match self {
            Self::Utf8(ftp) => {
                let data = if verb == "RETR" {
                    ftp.retr_as_stream(path)
                } else {
                    ftp.put_with_stream(path)
                }
                .map_err(err)?;
                set_timeout(data.get_ref())?;
                Ok(Box::new(SdkData(data)))
            }
            Self::Encoded(ftp) => Ok(Box::new(
                ftp.passive_data(&format!("{verb} {path}")).map_err(err)?,
            )),
        }
    }
    fn finalize_retr_stream(&mut self, data: impl Read) -> FtpResult<()> {
        drop(data);
        match self {
            Self::Utf8(ftp) => ftp.finalize_retr_stream(std::io::empty()),
            Self::Encoded(ftp) => ftp
                .response(&[Status::ClosingDataConnection, Status::RequestedFileActionOk])
                .map(|_| ()),
        }
    }
    fn finalize_put_stream(&mut self, mut data: Box<dyn TransferStream>) -> FtpResult<()> {
        data.finish_write().map_err(FtpError::ConnectionError)?;
        drop(data);
        match self {
            Self::Utf8(ftp) => ftp.finalize_put_stream(std::io::sink()),
            Self::Encoded(ftp) => ftp
                .response(&[Status::ClosingDataConnection, Status::RequestedFileActionOk])
                .map(|_| ()),
        }
    }
}

enum Backend {
    Ftp(Box<Ftp>),
    Sftp { sftp: Sftp, session: Session },
}

/// A session belongs to one worker. Failed FTP transfers invalidate the session
/// because an unread completion reply could otherwise be used for the next command.
pub struct Client {
    backend: Option<Backend>,
    warnings: Vec<String>,
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn validate_text(value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        Err("Remote paths and commands cannot contain control characters".into())
    } else {
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    validate_text(name)?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\u{fffd}')
    {
        Err(format!("Unsafe or undecodable remote entry name: {name:?}"))
    } else {
        Ok(())
    }
}

fn validate_local_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if reserved || name.ends_with([' ', '.']) || name.chars().any(|c| "\\:<>\"|?*".contains(c)) {
        return Err(format!(
            "Remote name cannot be represented exactly as a Windows filename: {name:?}"
        ));
    }
    Ok(())
}

fn local_regular_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "Symbolic links are not followed: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(format!(
                "Reparse points are not followed: {}",
                path.display()
            ));
        }
    }
    Ok(meta)
}

struct LocalTree(PathBuf);
impl Drop for LocalTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn remote_child(parent: &str, name: &str) -> String {
    format!(
        "{}{name}",
        if parent.ends_with('/') {
            parent.to_owned()
        } else {
            format!("{parent}/")
        }
    )
}

fn unique_local_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination.parent().unwrap_or(Path::new("."));
    let name = destination
        .file_name()
        .ok_or("Missing destination name")?
        .to_string_lossy();
    for number in 1..100_000 {
        let candidate = parent.join(format!("{name} ({number})"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err("Could not choose an unused local name".into())
}

fn sftp_path(path: &str) -> Result<&Path, String> {
    validate_text(path)?;
    // ssh2 on Windows rewrites backslash to slash. Reject instead of silently
    // accessing a different server object. Do not use Windows path normalization.
    if cfg!(windows) && path.contains('\\') {
        return Err("Literal backslash in SFTP paths is unsupported on Windows by ssh2".into());
    }
    Ok(Path::new(path))
}

fn normalize_fingerprint(value: &str) -> Result<String, String> {
    let value = value.trim();
    let digest = value
        .strip_prefix("SHA256:")
        .ok_or("Expected SHA256 host fingerprint")?;
    let decoded = STANDARD_NO_PAD
        .decode(digest)
        .or_else(|_| STANDARD.decode(digest))
        .map_err(|_| "Invalid SHA256 host fingerprint")?;
    if decoded.len() != 32 {
        return Err("SHA256 host fingerprint must contain 32 bytes".into());
    }
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(decoded)))
}

fn trusted_fingerprint(expected: &str, candidate: &str) -> bool {
    let Ok(expected) = normalize_fingerprint(expected) else {
        return false;
    };
    // Both canonical SHA256 values have fixed length. No data-dependent early exit.
    expected
        .as_bytes()
        .iter()
        .zip(candidate.as_bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
        && expected.len() == candidate.len()
}

fn socket(host: &str, port: u16) -> Result<TcpStream, String> {
    let mut last = "Host did not resolve to an address".to_owned();
    let deadline = Instant::now() + TIMEOUT;
    for address in (host, port).to_socket_addrs().map_err(err)? {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                set_timeout(&stream)?;
                return Ok(stream);
            }
            Err(error) => last = error.to_string(),
        }
    }
    Err(last)
}

fn set_timeout(stream: &TcpStream) -> Result<(), String> {
    // Windows accepted sockets inherit the listener's nonblocking mode.
    stream.set_nonblocking(false).map_err(err)?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(err)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(err)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Transfer cancelled".into())
    } else {
        Ok(())
    }
}

fn copy_cancel(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let mut buffer = [0u8; 128 * 1024];
    let mut total = 0;
    loop {
        check_cancel(cancel)?;
        let count = reader.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        check_cancel(cancel)?;
        writer.write_all(&buffer[..count]).map_err(err)?;
        total += count as u64;
    }
    check_cancel(cancel)?;
    writer.flush().map_err(err)?;
    Ok(total)
}

fn temp_name() -> String {
    format!(
        ".ndr-transfer-{}-{:x}-{:x}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )
}

fn remote_temp(path: &str) -> String {
    let prefix = path.rfind('/').map(|at| &path[..=at]).unwrap_or("");
    format!("{prefix}{}", temp_name())
}

struct LocalTemp(PathBuf);
impl Drop for LocalTemp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl LocalTemp {
    fn create(destination: &Path) -> Result<(Self, File), String> {
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let path = parent.join(temp_name());
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(err)?;
        Ok((Self(path), file))
    }
    fn commit(self, destination: &Path) -> Result<(), String> {
        if let Ok(meta) = fs::symlink_metadata(destination) {
            if meta.is_dir() || meta.file_type().is_symlink() {
                return Err("Refusing to replace a directory or symbolic link".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("Refusing to replace a reparse point".into());
                }
            }
        }
        // Same-directory rename replaces the existing regular file atomically.
        fs::rename(&self.0, destination).map_err(err)
    }
}

impl Client {
    /// Returns and clears nonfatal metadata warnings from completed transfers.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
    fn directory_times(&mut self, remote: &str) -> Result<Option<(i64, i64)>, String> {
        match self.backend()? {
            Backend::Ftp(_) => Ok(None), // FTP MDTM is not portable for directories.
            Backend::Sftp { sftp, .. } => {
                let stat = sftp.lstat(sftp_path(remote)?).map_err(err)?;
                if !stat.is_dir() || stat.file_type().is_symlink() {
                    return Err("Directory changed type during metadata read".into());
                }
                Ok(stat.mtime.and_then(|t| i64::try_from(t).ok()).map(|mtime| {
                    (
                        stat.atime
                            .and_then(|t| i64::try_from(t).ok())
                            .unwrap_or(mtime),
                        mtime,
                    )
                }))
            }
        }
    }
    fn preserve_local_directory_times(
        &mut self,
        path: &Path,
        times: (i64, i64),
    ) -> Result<(), String> {
        if !local_regular_metadata(path)?.is_dir() {
            return Err("Local directory changed type during metadata update".into());
        }
        if let Err(error) = filetime::set_file_times(
            path,
            filetime::FileTime::from_unix_time(times.0, 0),
            filetime::FileTime::from_unix_time(times.1, 0),
        ) {
            self.warnings.push(format!(
                "Directory timestamp could not be preserved for {}: {error}",
                path.display()
            ));
        }
        Ok(())
    }
    fn preserve_remote_directory_times(
        &mut self,
        remote: &str,
        meta: &fs::Metadata,
    ) -> Result<(), String> {
        let atime = filetime::FileTime::from_last_access_time(meta).unix_seconds();
        let mtime = filetime::FileTime::from_last_modification_time(meta).unix_seconds();
        let result = match self.backend()? {
            Backend::Ftp(_) => return Ok(()),
            Backend::Sftp { sftp, .. } => {
                let path = sftp_path(remote)?;
                let stat = sftp.lstat(path).map_err(err)?;
                if !stat.is_dir() || stat.file_type().is_symlink() {
                    return Err("Remote directory changed type during metadata update".into());
                }
                match (u64::try_from(atime), u64::try_from(mtime)) {
                    (Ok(atime), Ok(mtime)) => sftp.setstat(
                        path,
                        ssh2::FileStat {
                            size: None,
                            uid: None,
                            gid: None,
                            perm: None,
                            atime: Some(atime),
                            mtime: Some(mtime),
                        },
                    ),
                    _ => {
                        self.warnings.push(format!(
                            "SFTP cannot preserve pre-epoch directory timestamps for {remote}"
                        ));
                        return Ok(());
                    }
                }
            }
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if matches!(error.code(), ssh2::ErrorCode::SFTP(_)) => {
                self.warnings.push(format!(
                    "Directory timestamp could not be preserved for {remote}: {error}"
                ));
                Ok(())
            }
            Err(error) => Err(err(error)),
        }
    }
    /// Overwrite merges directories, asking again for each colliding child.
    /// ReplaceFolder stages the entire replacement before moving the old tree
    /// to a backup. Cancellation during a merge preserves each uncommitted file.
    pub fn download_tree_with_conflicts(
        &mut self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        resolve: &mut dyn FnMut(&str, &str, &ConflictInfo) -> crate::fsops::ConflictChoice,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<(), String> {
        use crate::fsops::ConflictChoice;
        let mut pending = vec![(remote.to_owned(), local.to_owned())];
        let mut merged_directory_times = Vec::new();
        let (mut bytes, mut files) = (0, 0);
        while let Some((source, mut destination)) = pending.pop() {
            check_cancel(cancel)?;
            let entry = self
                .remote_entry(&source)?
                .ok_or("Remote source disappeared")?;
            if entry.is_symlink {
                return Err(format!("Symbolic links are not followed: {source}"));
            }
            if fs::symlink_metadata(&destination).is_ok() {
                let meta = local_regular_metadata(&destination)?;
                let can_resume = !entry.is_dir && meta.is_file() && self.can_resume();
                let info = ConflictInfo { remote: &entry, local_dir: meta.is_dir(), remote_is_source: true, can_resume, new_name: Default::default() };
                let mut choice = resolve(&source, &destination.to_string_lossy(), &info);
                if choice == ConflictChoice::Rename && let Some(name) = info.new_name.take() {
                    // Original FUN_00469210 (0x10): after renaming to the typed name, the same item runs again (asking again when that name exists too).
                    validate_local_name(&name)?;
                    pending.push((source, destination.with_file_name(name)));
                    continue;
                }
                if choice == ConflictChoice::OverwriteNewer {
                    choice = newer_wins(entry.modified, local_seconds(&meta));
                }
                if choice == ConflictChoice::Resume {
                    // Original FUN_00467f0c (+0x40): receives from the size of the local partial file and appends. Overwrites when resuming is impossible.
                    if can_resume && meta.len() < entry.size {
                        self.download_resume(&source, &destination, cancel)?;
                        bytes += entry.size;
                        files += 1;
                        progress(TransferProgress { path: source, bytes, files });
                        continue;
                    }
                    choice = ConflictChoice::Overwrite;
                }
                match choice {
                    ConflictChoice::Cancel => return Err("Transfer cancelled".into()),
                    ConflictChoice::Skip => continue,
                    ConflictChoice::OverwriteNewer | ConflictChoice::Resume => unreachable!("handled above"),
                    ConflictChoice::Rename => destination = unique_local_path(&destination)?,
                    ConflictChoice::Overwrite if entry.is_dir && meta.is_dir() => {
                        if let Some(times) = self.directory_times(&source)? {
                            merged_directory_times.push((destination.clone(), times));
                        }
                        for child in self.list_nofollow(&source)? {
                            validate_local_name(&child.name)?;
                            if child.is_symlink {
                                return Err(format!(
                                    "Symbolic links are not followed: {}",
                                    child.name
                                ));
                            }
                            pending.push((
                                remote_child(&source, &child.name),
                                destination.join(&child.name),
                            ));
                        }
                        continue;
                    }
                    ConflictChoice::Overwrite if !entry.is_dir && meta.is_file() => {
                        self.download(&source, &destination, cancel)?;
                        bytes += entry.size;
                        files += 1;
                        progress(TransferProgress {
                            path: source,
                            bytes,
                            files,
                        });
                        continue;
                    }
                    ConflictChoice::Overwrite => {
                        return Err("File/directory type conflict requires ReplaceFolder".into());
                    }
                    ConflictChoice::ReplaceFolder => {
                        let parent = destination.parent().unwrap_or(Path::new("."));
                        let stage = LocalTree(parent.join(temp_name()));
                        let start = (bytes, files);
                        let mut delta = (0, 0);
                        self.download_tree_with_progress(&source, &stage.0, cancel, &mut |p| {
                            delta = (p.bytes, p.files);
                            progress(TransferProgress {
                                path: p.path,
                                bytes: start.0 + p.bytes,
                                files: start.1 + p.files,
                            });
                        })?;
                        check_cancel(cancel)?;
                        let backup = parent.join(temp_name());
                        fs::rename(&destination, &backup).map_err(err)?;
                        if let Err(commit) = fs::rename(&stage.0, &destination) {
                            return match fs::rename(&backup, &destination) {
                                Ok(()) => Err(err(commit)),
                                Err(restore) => Err(format!(
                                    "Replacement failed: {commit}; restore failed: {restore}; original retained at {}",
                                    backup.display()
                                )),
                            };
                        }
                        let cleanup = if meta.is_dir() {
                            fs::remove_dir_all(&backup)
                        } else {
                            fs::remove_file(&backup)
                        };
                        cleanup.map_err(|e| {
                            format!(
                                "Replacement committed; old backup retained at {}: {e}",
                                backup.display()
                            )
                        })?;
                        bytes += delta.0;
                        files += delta.1;
                        continue;
                    }
                }
            }
            let start = (bytes, files);
            let mut delta = (0, 0);
            self.download_tree_with_progress(&source, &destination, cancel, &mut |p| {
                delta = (p.bytes, p.files);
                progress(TransferProgress {
                    path: p.path,
                    bytes: start.0 + p.bytes,
                    files: start.1 + p.files,
                });
            })?;
            bytes += delta.0;
            files += delta.1;
        }
        for (path, times) in merged_directory_times.into_iter().rev() {
            self.preserve_local_directory_times(&path, times)?;
        }
        Ok(())
    }

    pub fn upload_tree_with_conflicts(
        &mut self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        resolve: &mut dyn FnMut(&str, &str, &ConflictInfo) -> crate::fsops::ConflictChoice,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<(), String> {
        use crate::fsops::ConflictChoice;
        let mut pending = vec![(local.to_owned(), remote.to_owned())];
        let mut merged_directory_times = Vec::new();
        let (mut bytes, mut files) = (0, 0);
        while let Some((source, mut destination)) = pending.pop() {
            check_cancel(cancel)?;
            let meta = local_regular_metadata(&source)?;
            if let Some(entry) = self.remote_entry(&destination)? {
                let can_resume = meta.is_file() && !entry.is_dir && !entry.is_symlink && self.can_resume();
                let info = ConflictInfo { remote: &entry, local_dir: meta.is_dir(), remote_is_source: false, can_resume, new_name: Default::default() };
                let mut choice = resolve(&source.to_string_lossy(), &destination, &info);
                if choice == ConflictChoice::Rename && let Some(name) = info.new_name.take() {
                    validate_name(&name)?;
                    let parent = destination.rsplit_once('/').map_or("", |(parent, _)| parent).to_owned();
                    pending.push((source, remote_child(if parent.is_empty() { "/" } else { &parent }, &name)));
                    continue;
                }
                if choice == ConflictChoice::OverwriteNewer {
                    choice = newer_wins(local_seconds(&meta), entry.modified);
                }
                if choice == ConflictChoice::Resume {
                    // Original FUN_0087c2f0 (Synapse StoreFile Restore) and SFTP PutFile (resume): appends the rest after the remote partial file.
                    if can_resume && entry.size < meta.len() {
                        self.upload_resume(&source, &destination, cancel)?;
                        bytes += meta.len();
                        files += 1;
                        progress(TransferProgress { path: destination, bytes, files });
                        continue;
                    }
                    choice = ConflictChoice::Overwrite;
                }
                match choice {
                    ConflictChoice::Cancel => return Err("Transfer cancelled".into()),
                    ConflictChoice::Skip => continue,
                    ConflictChoice::OverwriteNewer | ConflictChoice::Resume => unreachable!("handled above"),
                    ConflictChoice::Rename => {
                        destination = self.unique_remote_path(&destination)?
                    }
                    ConflictChoice::Overwrite
                        if meta.is_dir() && entry.is_dir && !entry.is_symlink =>
                    {
                        merged_directory_times.push((destination.clone(), meta));
                        for child in fs::read_dir(&source).map_err(err)? {
                            let child = child.map_err(err)?;
                            let name = child
                                .file_name()
                                .into_string()
                                .map_err(|_| "Non-Unicode local filename")?;
                            validate_name(&name)?;
                            local_regular_metadata(&child.path())?;
                            pending.push((child.path(), remote_child(&destination, &name)));
                        }
                        continue;
                    }
                    ConflictChoice::Overwrite if meta.is_file() && !entry.is_dir => {
                        self.upload(&source, &destination, cancel)?;
                        bytes += meta.len();
                        files += 1;
                        progress(TransferProgress {
                            path: destination,
                            bytes,
                            files,
                        });
                        continue;
                    }
                    ConflictChoice::Overwrite => {
                        return Err("File/directory type conflict requires ReplaceFolder".into());
                    }
                    ConflictChoice::ReplaceFolder => {
                        let stage = remote_temp(&destination);
                        let start = (bytes, files);
                        let mut delta = (0, 0);
                        self.upload_tree_with_progress(&source, &stage, cancel, &mut |p| {
                            delta = (p.bytes, p.files);
                            progress(TransferProgress {
                                path: p.path,
                                bytes: start.0 + p.bytes,
                                files: start.1 + p.files,
                            });
                        })?;
                        let backup = remote_temp(&destination);
                        let commit = (|| {
                            check_cancel(cancel)?;
                            self.rename(&destination, &backup)?;
                            if let Err(commit) = self.rename(&stage, &destination) {
                                return match self.rename(&backup, &destination) {
                                    Ok(()) => Err(commit),
                                    Err(restore) => Err(format!(
                                        "Replacement failed: {commit}; restore failed: {restore}; original retained at {backup}"
                                    )),
                                };
                            }
                            self.delete_tree(&backup,&AtomicBool::new(false)).map_err(|e|format!("Replacement committed; old backup retained at {backup}: {e}"))
                        })();
                        if commit.is_err() {
                            let _ = self.delete_tree(&stage, &AtomicBool::new(false));
                        }
                        commit?;
                        bytes += delta.0;
                        files += delta.1;
                        continue;
                    }
                }
            }
            let start = (bytes, files);
            let mut delta = (0, 0);
            self.upload_tree_with_progress(&source, &destination, cancel, &mut |p| {
                delta = (p.bytes, p.files);
                progress(TransferProgress {
                    path: p.path,
                    bytes: start.0 + p.bytes,
                    files: start.1 + p.files,
                });
            })?;
            bytes += delta.0;
            files += delta.1;
        }
        for (path, meta) in merged_directory_times.into_iter().rev() {
            self.preserve_remote_directory_times(&path, &meta)?;
        }
        Ok(())
    }

    fn unique_remote_path(&mut self, destination: &str) -> Result<String, String> {
        for number in 1..100_000 {
            let candidate = format!("{destination} ({number})");
            if self.remote_entry(&candidate)?.is_none() {
                return Ok(candidate);
            }
        }
        Err("Could not choose an unused remote name".into())
    }
    /// `mode` is a Unix bit mask (e.g. 0o644), not the decimal number 644.
    pub fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        validate_text(path)?;
        if mode > 0o7777 {
            return Err("Permission mode must be within 0000..7777 octal".into());
        }
        let entry = self
            .remote_entry(path)?
            .ok_or("Permission target does not exist")?;
        if entry.is_symlink {
            return Err("Changing permissions through a symbolic link is refused".into());
        }
        match self.backend()? {
            Backend::Ftp(ftp) => ftp
                .stream
                .site(format!("CHMOD {mode:03o} {path}"))
                .map(|_| ())
                .map_err(err),
            Backend::Sftp { sftp, .. } => sftp
                .setstat(
                    sftp_path(path)?,
                    ssh2::FileStat {
                        size: None,
                        uid: None,
                        gid: None,
                        perm: Some(mode),
                        atime: None,
                        mtime: None,
                    },
                )
                .map_err(err),
        }
    }
    pub fn permissions(&mut self, path: &str) -> Result<Option<u32>, String> {
        validate_text(path)?;
        match self.backend()? {
            Backend::Sftp { sftp, .. } => Ok(sftp
                .lstat(sftp_path(path)?)
                .map_err(err)?
                .perm
                .map(|p| p & 0o7777)),
            Backend::Ftp(ftp) => {
                let response = match ftp.stream.mlst(Some(path)) {
                    Ok(response) => response,
                    Err(FtpError::UnexpectedResponse(response))
                        if matches!(response.status as u32, 500 | 502 | 504) =>
                    {
                        return Ok(None);
                    }
                    Err(e) => return Err(err(e)),
                };
                let lower = response.to_ascii_lowercase();
                let Some(start) = lower.find("unix.mode=") else {
                    return Ok(None);
                };
                let value = lower[start + 10..]
                    .split([';', ' ', '\r', '\n'])
                    .next()
                    .unwrap_or("");
                Ok(Some(u32::from_str_radix(value, 8).map_err(err)? & 0o7777))
            }
        }
    }
    /// Stages a new tree beside `local`. Existing destinations are never merged
    /// or replaced. Links and names Windows cannot represent abort the transfer.
    pub fn download_tree(
        &mut self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.download_tree_with_progress(remote, local, cancel, &mut |_| {})
    }
    pub fn download_tree_with_progress(
        &mut self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        if fs::symlink_metadata(local).is_ok() {
            return Err("Tree download destination already exists".into());
        }
        let entry = self
            .remote_entry(remote)?
            .ok_or("Remote source does not exist")?;
        if entry.is_symlink {
            return Err("Symbolic links are not followed during tree downloads".into());
        }
        if !entry.is_dir {
            self.download(remote, local, cancel)?;
            progress(TransferProgress {
                path: remote.into(),
                bytes: entry.size,
                files: 1,
            });
            return Ok(());
        }
        let parent = local
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = LocalTree(parent.join(temp_name()));
        fs::create_dir(&temporary.0).map_err(err)?;
        let mut stack = vec![(remote.to_owned(), temporary.0.clone())];
        let mut directory_times = Vec::new();
        let (mut bytes, mut files) = (0, 0);
        while let Some((remote, destination)) = stack.pop() {
            check_cancel(cancel)?;
            let current = self
                .remote_entry(&remote)?
                .ok_or("Remote directory disappeared during transfer")?;
            if current.is_symlink || !current.is_dir {
                return Err("Remote directory changed type during transfer".into());
            }
            if let Some(times) = self.directory_times(&remote)? {
                directory_times.push((destination.clone(), times));
            }
            for entry in self.list_nofollow(&remote)? {
                check_cancel(cancel)?;
                validate_local_name(&entry.name)?;
                if entry.is_symlink {
                    return Err(format!("Symbolic link is not followed: {}", entry.name));
                }
                let child = remote_child(&remote, &entry.name);
                let target = destination.join(&entry.name);
                if fs::symlink_metadata(&target).is_ok() {
                    return Err(format!(
                        "Remote names collide on local filesystem: {}",
                        entry.name
                    ));
                }
                if entry.is_dir {
                    fs::create_dir(&target).map_err(err)?;
                    stack.push((child, target));
                } else {
                    self.download(&child, &target, cancel)?;
                    bytes += entry.size;
                    files += 1;
                    progress(TransferProgress {
                        path: child,
                        bytes,
                        files,
                    });
                }
            }
        }
        for (path, times) in directory_times.into_iter().rev() {
            self.preserve_local_directory_times(&path, times)?;
        }
        check_cancel(cancel)?;
        if fs::symlink_metadata(local).is_ok() {
            return Err("Tree download destination appeared during transfer".into());
        }
        fs::rename(&temporary.0, local).map_err(err)
    }

    pub fn upload_tree(
        &mut self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.upload_tree_with_progress(local, remote, cancel, &mut |_| {})
    }
    pub fn upload_tree_with_progress(
        &mut self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let meta = local_regular_metadata(local)?;
        if self.remote_entry(remote)?.is_some() {
            return Err("Tree upload destination already exists".into());
        }
        if meta.is_file() {
            self.upload(local, remote, cancel)?;
            progress(TransferProgress {
                path: remote.into(),
                bytes: meta.len(),
                files: 1,
            });
            return Ok(());
        }
        if !meta.is_dir() {
            return Err("Upload source is not a regular file or directory".into());
        }
        let temporary = remote_temp(remote);
        self.mkdir(&temporary)?;
        let result = (|| {
            let mut stack = vec![(local.to_owned(), temporary.clone())];
            let mut directory_times = Vec::new();
            let (mut bytes, mut files) = (0, 0);
            while let Some((source, destination)) = stack.pop() {
                check_cancel(cancel)?;
                let meta = local_regular_metadata(&source)?;
                if !meta.is_dir() {
                    return Err("Local directory changed type during transfer".into());
                }
                directory_times.push((destination.clone(), meta));
                for entry in fs::read_dir(&source).map_err(err)? {
                    check_cancel(cancel)?;
                    let entry = entry.map_err(err)?;
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "Non-Unicode local filename")?;
                    validate_name(&name)?;
                    let target = remote_child(&destination, &name);
                    let meta = local_regular_metadata(&entry.path())?;
                    if meta.is_dir() {
                        self.mkdir(&target)?;
                        stack.push((entry.path(), target));
                    } else if meta.is_file() {
                        self.upload(&entry.path(), &target, cancel)?;
                        bytes += meta.len();
                        files += 1;
                        progress(TransferProgress {
                            path: target,
                            bytes,
                            files,
                        });
                    } else {
                        return Err(format!(
                            "Unsupported local entry: {}",
                            entry.path().display()
                        ));
                    }
                }
            }
            for (path, meta) in directory_times.into_iter().rev() {
                self.preserve_remote_directory_times(&path, &meta)?;
            }
            check_cancel(cancel)?;
            if self.remote_entry(remote)?.is_some() {
                return Err("Tree upload destination appeared during transfer".into());
            }
            self.rename(&temporary, remote)
        })();
        if let Err(error) = result {
            let cleanup = self.delete_tree(&temporary, &AtomicBool::new(false));
            return Err(match cleanup {
                Ok(()) => error,
                Err(e) => format!("{error}; temporary tree retained at {temporary}: {e}"),
            });
        }
        Ok(())
    }

    pub fn delete_tree(&mut self, remote: &str, cancel: &AtomicBool) -> Result<(), String> {
        self.delete_tree_with_progress(remote, cancel, &mut |_| {})
    }
    /// Original TNXFileListAdapter FUN_0045d370 (panel FTP/SFTP folder delete): enters the folder, deletes its files, recurses into each
    /// subfolder except "." and "..", removes it (FTP RemoveDir / SFTP DeleteDir) and returns to the previous folder. Same result here with
    /// absolute paths; links are deleted without being followed, and a root or dot path is refused.
    pub fn delete_tree_with_progress(
        &mut self,
        remote: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<(), String> {
        validate_text(remote)?;
        if remote.trim_matches('/').is_empty()
            || remote.split('/').any(|part| part == "." || part == "..")
        {
            return Err("Refusing recursive delete of a root or dot path".into());
        }
        let mut stack = vec![(remote.trim_end_matches('/').to_owned(), false)];
        let (mut bytes, mut files) = (0, 0);
        while let Some((path, after_children)) = stack.pop() {
            check_cancel(cancel)?;
            let entry = self
                .remote_entry(&path)?
                .ok_or("Remote entry disappeared during delete")?;
            if entry.is_dir && !entry.is_symlink {
                if after_children {
                    self.delete(&path, true)?;
                } else {
                    let children = self.list_nofollow(&path)?;
                    stack.push((path.clone(), true));
                    for entry in children {
                        stack.push((remote_child(&path, &entry.name), false));
                    }
                }
            } else {
                self.delete(&path, false)?;
                bytes += entry.size;
                files += 1;
                progress(TransferProgress { path, bytes, files });
            }
        }
        Ok(())
    }

    fn list_nofollow(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        match self.backend()? {
            Backend::Ftp(ftp) => ftp.list_nofollow(path),
            Backend::Sftp { .. } => self.list(path),
        }
    }

    fn remote_entry(&mut self, path: &str) -> Result<Option<RemoteEntry>, String> {
        validate_text(path)?;
        match self.backend()? {
            Backend::Sftp { sftp, .. } => match sftp.lstat(sftp_path(path)?) {
                Ok(stat) => Ok(Some(RemoteEntry {
                    name: path.rsplit('/').next().unwrap_or("").into(),
                    is_dir: stat.is_dir(),
                    size: stat.size.unwrap_or(0),
                    is_symlink: stat.file_type().is_symlink(),
                    modified: stat.mtime.and_then(|t| i64::try_from(t).ok()),
                    mode: stat.perm.map(|p| p & 0o7777),
                    permission: None,
                })),
                Err(e) if matches!(e.code(), ssh2::ErrorCode::SFTP(2 | 10)) => Ok(None),
                Err(e) => Err(err(e)),
            },
            Backend::Ftp(ftp) => {
                let path = path.trim_end_matches('/');
                if path.is_empty() {
                    return Ok(Some(RemoteEntry {
                        name: "/".into(),
                        is_dir: true,
                        ..Default::default()
                    }));
                }
                let (parent, name) = path.rsplit_once('/').unwrap_or((".", path));
                validate_name(name)?;
                Ok(ftp
                    .list_nofollow(if parent.is_empty() { "/" } else { parent })?
                    .into_iter()
                    .find(|entry| entry.name == name))
            }
        }
    }
    pub fn connect(config: &Config) -> Result<Self, String> {
        Self::connect_inner(config).map_err(|mut error| {
            // The host-key challenge contains no credentials. Preserve its
            // machine-readable candidate even if a short password happens to
            // occur in the public fingerprint's base64 representation.
            if serde_json::from_str::<serde_json::Value>(&error)
                .ok()
                .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
                .is_some_and(|kind| kind == "unknown_host_key" || kind == "host_key_mismatch")
            {
                return error;
            }
            for secret in [&config.password, &config.key_passphrase] {
                if !secret.is_empty() {
                    error = error.replace(secret, "[redacted]");
                }
            }
            error
        })
    }

    fn connect_inner(config: &Config) -> Result<Self, String> {
        for value in [
            &config.host,
            &config.user,
            &config.password,
            &config.key_passphrase,
        ] {
            validate_text(value)?;
        }
        if config.host.is_empty() {
            return Err("Host is required".into());
        }
        let protocol = config.protocol.to_ascii_lowercase();
        if !["ftp", "ftps", "sftp"].contains(&protocol.as_str()) {
            return Err(format!("Unsupported remote protocol: {}", config.protocol));
        }
        if config.port == 0 {
            return Err("Remote port must be between 1 and 65535".into());
        }
        let port = config.port;
        let stream = socket(&config.host, port)?;
        let backend = match protocol.as_str() {
            "ftp" | "ftps" => {
                let label = config.encoding_name.trim().to_ascii_lowercase();
                let label = match label.as_str() {
                    "" => "utf-8",
                    "949" | "cp949" | "windows-949" => "euc-kr",
                    "932" | "cp932" => "shift_jis",
                    "936" | "cp936" => "gbk",
                    "950" | "cp950" => "big5",
                    _ => &label,
                };
                let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
                    .ok_or("Unsupported FTP encoding label")?;
                if !encoding.is_ascii_compatible() {
                    return Err("FTP encoding must preserve ASCII command framing".into());
                }
                let tls = if protocol == "ftps" {
                    // Native trust store and hostname checks are both enabled by default.
                    Some(native_tls::TlsConnector::new().map_err(err)?)
                } else {
                    None
                };
                Backend::Ftp(Box::new(Ftp::connect(config, stream, encoding, tls)?))
            }
            "sftp" => {
                let mut session = Session::new().map_err(err)?;
                session.set_timeout(TIMEOUT.as_millis() as u32);
                session.set_tcp_stream(stream);
                session.handshake().map_err(err)?;
                let (key, _) = session
                    .host_key()
                    .ok_or("SSH server provided no host key")?;
                let candidate = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(key)));
                if !trusted_fingerprint(&config.fingerprint, &candidate) {
                    return Err(serde_json::json!({
                        "kind": if config.fingerprint.trim().is_empty() { "unknown_host_key" } else { "host_key_mismatch" },
                        "host": config.host, "port": port, "candidate": candidate,
                        "message": "Verify this SHA256 fingerprint through a trusted channel before accepting it"
                    }).to_string());
                }
                if config.private_key.as_os_str().is_empty() {
                    session
                        .userauth_password(&config.user, &config.password)
                        .map_err(err)?;
                } else {
                    let passphrase = if config.key_passphrase.is_empty() {
                        None
                    } else {
                        Some(config.key_passphrase.as_str())
                    };
                    session
                        .userauth_pubkey_file(&config.user, None, &config.private_key, passphrase)
                        .map_err(err)?;
                }
                if !session.authenticated() {
                    return Err("SSH authentication failed".into());
                }
                let sftp = session.sftp().map_err(err)?;
                Backend::Sftp { sftp, session }
            }
            _ => return Err(format!("Unsupported remote protocol: {}", config.protocol)),
        };
        Ok(Self {
            backend: Some(backend),
            warnings: Vec::new(),
        })
    }

    fn backend(&mut self) -> Result<&mut Backend, String> {
        self.backend
            .as_mut()
            .ok_or("Connection is closed; reconnect before continuing".into())
    }

    pub fn pwd(&mut self) -> Result<String, String> {
        match self.backend()? {
            Backend::Ftp(ftp) => ftp.stream.pwd().map_err(err),
            Backend::Sftp { sftp, .. } => sftp
                .realpath(Path::new("."))
                .map_err(err)?
                .into_os_string()
                .into_string()
                .map_err(|_| "SFTP returned a non-Unicode path".into()),
        }
    }

    /// C# RemoteWindow.NavigateAsync. FTP sends the path as is as "CWD path" (CDUP for ".."), and even when refused reads the server-chosen path with PWD
    /// (second value = the refusal reply text). SFTP resolves it relative to the current path like SSH.NET ChangeDirectory (realpath)
    /// and checks that it opens as a folder.
    pub fn change_directory(&mut self, current: &str, path: &str) -> Result<(String, Option<String>), String> {
        validate_text(path)?;
        if matches!(self.backend()?, Backend::Ftp(_)) {
            let command = if path == ".." { "CDUP".to_owned() } else { format!("CWD {path}") };
            let rejected = self.command(&command).err();
            return Ok((self.pwd()?, rejected));
        }
        let Backend::Sftp { sftp, .. } = self.backend()? else {
            return Err("Connection is closed; reconnect before continuing".into());
        };
        let target = if path.is_empty() { "." } else { path };
        let joined = if target.starts_with('/') { target.to_owned() } else { format!("{}/{}", current.trim_end_matches('/'), target) };
        let real = sftp.realpath(sftp_path(&joined)?).map_err(err)?;
        let mut directory = sftp.opendir(&real).map_err(err)?;
        let _ = directory.close();
        let real = real.into_os_string().into_string().map_err(|_| "SFTP returned a non-Unicode path")?;
        Ok((real, None))
    }

    /// C# ExecuteSshCommand (SshClient.RunCommand(command).Result): runs one command on the SSH session of the SFTP connection and returns standard output.
    pub fn exec(&mut self, command: &str) -> Result<String, String> {
        validate_text(command)?;
        match self.backend()? {
            Backend::Sftp { session, .. } => {
                let mut channel = session.channel_session().map_err(err)?;
                channel.exec(command).map_err(err)?;
                let mut output = Vec::new();
                channel.read_to_end(&mut output).map_err(err)?;
                let _ = channel.wait_close();
                Ok(String::from_utf8_lossy(&output).into_owned())
            }
            Backend::Ftp(_) => Err("SSH commands need an SFTP connection".into()),
        }
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        validate_text(path)?;
        match self.backend()? {
            Backend::Ftp(ftp) => ftp.list(path),
            Backend::Sftp { sftp, .. } => {
                let mut directory = sftp.opendir(sftp_path(path)?).map_err(err)?;
                let mut entries = Vec::new();
                loop {
                    match directory.readdir() {
                        Ok((name, stat)) => {
                            let name = name
                                .into_os_string()
                                .into_string()
                                .map_err(|_| "SFTP returned a non-Unicode name")?;
                            if name == "." || name == ".." {
                                continue;
                            }
                            validate_name(&name)?;
                            entries.push(RemoteEntry {
                                name,
                                is_dir: stat.is_dir(),
                                size: stat.size.unwrap_or(0),
                                is_symlink: stat.file_type().is_symlink(),
                                modified: stat.mtime.and_then(|t| i64::try_from(t).ok()),
                                mode: stat.perm.map(|p| p & 0o7777),
                                permission: None,
                            });
                        }
                        Err(e) if e.code() == ssh2::ErrorCode::Session(-16) => break, // libssh2's readdir EOF.
                        Err(e) => return Err(err(e)),
                    }
                }
                directory.close().map_err(err)?;
                Ok(entries)
            }
        }
    }

    /// Original btnResume enabled: FTP asks whether REST is accepted like Synapse CanResume (true when "REST 1" gives 350 at login; here "REST 0",
    /// which does not affect the next transfer), SFTP always can.
    pub fn can_resume(&mut self) -> bool {
        match self.backend.as_mut() {
            Some(Backend::Ftp(ftp)) => ftp.stream.rest(0).is_ok(),
            Some(Backend::Sftp { .. }) => true,
            None => false,
        }
    }

    /// Original FUN_00467f0c (+0x40 resume download, start = local size): appends the rest of the remote file after the local partial file (FTP REST, SFTP seek).
    pub fn download_resume(&mut self, remote: &str, local: &Path, cancel: &AtomicBool) -> Result<(), String> {
        validate_text(remote)?;
        check_cancel(cancel)?;
        let meta = local_regular_metadata(local)?;
        if !meta.is_file() {
            return Err("Resume target must be a regular local file".into());
        }
        let offset = meta.len();
        let mut output = OpenOptions::new().append(true).open(local).map_err(err)?;
        let mut remote_times = None;
        let result: Result<(), String> = match self.backend()? {
            Backend::Ftp(ftp) => (|| {
                let size = ftp.stream.size(remote).ok().map(|n| n as u64);
                match ftp.stream.mtime(remote) {
                    Ok(mtime) => remote_times = Some((mtime, mtime)),
                    Err(FtpError::UnexpectedResponse(_)) => {}
                    Err(error) => return Err(err(error)),
                }
                ftp.stream.rest(offset).map_err(err)?;
                let mut data = ftp.data("RETR", remote)?;
                let copied = copy_cancel(&mut data, &mut output, cancel)?;
                ftp.stream.finalize_retr_stream(data).map_err(err)?;
                if size.is_some_and(|size| size != offset + copied) {
                    return Err("Resumed download size did not match remote file".into());
                }
                Ok(())
            })(),
            Backend::Sftp { sftp, .. } => (|| {
                let path = sftp_path(remote)?;
                let stat = sftp.lstat(path).map_err(err)?;
                if !stat.is_file() {
                    return Err("Download source must be a regular file; symbolic links are not followed".into());
                }
                if let Some(mtime) = stat.mtime.and_then(|time| i64::try_from(time).ok()) {
                    remote_times = Some((stat.atime.and_then(|time| i64::try_from(time).ok()).unwrap_or(mtime), mtime));
                }
                let mut input = sftp.open(path).map_err(err)?;
                input.seek(SeekFrom::Start(offset)).map_err(err)?;
                let copied = copy_cancel(&mut input, &mut output, cancel)?;
                input.close().map_err(err)?;
                if stat.size.is_some_and(|size| size != offset + copied) {
                    return Err("Resumed download size did not match remote file".into());
                }
                Ok(())
            })(),
        };
        if result.is_err() && matches!(self.backend, Some(Backend::Ftp(_))) {
            self.invalidate();
        }
        result?;
        output.sync_all().map_err(err)?;
        drop(output);
        if let Some((atime, mtime)) = remote_times
            && let Err(error) = filetime::set_file_times(local, filetime::FileTime::from_unix_time(atime, 0), filetime::FileTime::from_unix_time(mtime, 0))
        {
            self.warnings.push(format!("Resumed bytes received; timestamps could not be preserved for {}: {error}", local.display()));
        }
        Ok(())
    }

    /// Original FUN_0087c2f0 (FTP resume upload = Synapse StoreFile Restore: REST remote size + STOR) and SFTP PutFile (resume):
    /// appends the rest of the local file after the remote partial file. It writes in place, so no temporary name is used.
    pub fn upload_resume(&mut self, local: &Path, remote: &str, cancel: &AtomicBool) -> Result<(), String> {
        validate_text(remote)?;
        check_cancel(cancel)?;
        let meta = local_regular_metadata(local)?;
        if !meta.is_file() {
            return Err("Upload source must be a regular file".into());
        }
        let existing = self.remote_entry(remote)?.ok_or("Remote resume target does not exist")?;
        if existing.is_dir || existing.is_symlink {
            return Err("Remote resume target must be a regular file".into());
        }
        let offset = existing.size;
        if offset > meta.len() {
            return Err("Remote file is larger than the local file".into());
        }
        let mut input = File::open(local).map_err(err)?;
        input.seek(SeekFrom::Start(offset)).map_err(err)?;
        let source_mtime = filetime::FileTime::from_last_modification_time(&meta).unix_seconds();
        let source_atime = filetime::FileTime::from_last_access_time(&meta).unix_seconds();
        let mut metadata_warning = None;
        let result: Result<(), String> = match self.backend()? {
            Backend::Ftp(ftp) => (|| {
                ftp.stream.rest(offset).map_err(err)?;
                let mut data = ftp.data("STOR", remote)?;
                let copied = copy_cancel(&mut input, &mut data, cancel)?;
                ftp.stream.finalize_put_stream(data).map_err(err)?;
                if offset + copied != meta.len() {
                    return Err("Upload source size changed during transfer".into());
                }
                if let Err(error) = ftp.stream.set_mtime(remote, source_mtime) {
                    if !matches!(error, FtpError::UnexpectedResponse(_)) {
                        return Err(err(error));
                    }
                    metadata_warning = Some(format!("Resumed bytes sent; modification time could not be preserved for {remote}: {error}"));
                }
                Ok(())
            })(),
            Backend::Sftp { sftp, .. } => (|| {
                let path = sftp_path(remote)?;
                let mut output = sftp.open_mode(path, OpenFlags::WRITE, 0o600, OpenType::File).map_err(err)?;
                output.seek(SeekFrom::Start(offset)).map_err(err)?;
                let copied = copy_cancel(&mut input, &mut output, cancel)?;
                output.close().map_err(err)?;
                if offset + copied != meta.len() || sftp.stat(path).map_err(err)?.size != Some(meta.len()) {
                    return Err("Resumed upload size verification failed".into());
                }
                if let (Ok(atime), Ok(mtime)) = (u64::try_from(source_atime), u64::try_from(source_mtime))
                    && let Err(error) = sftp.setstat(path, ssh2::FileStat { size: None, uid: None, gid: None, perm: None, atime: Some(atime), mtime: Some(mtime) })
                {
                    metadata_warning = Some(format!("Resumed bytes sent; timestamps could not be preserved for {remote}: {error}"));
                }
                Ok(())
            })(),
        };
        if let Some(warning) = metadata_warning {
            self.warnings.push(warning);
        }
        if result.is_err() && matches!(self.backend, Some(Backend::Ftp(_))) {
            self.invalidate();
        }
        result
    }

    pub fn download(
        &mut self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        validate_text(remote)?;
        check_cancel(cancel)?;
        validate_local_name(remote.rsplit('/').next().unwrap_or(""))?;
        if matches!(self.backend, Some(Backend::Ftp(_))) {
            let entry = self
                .remote_entry(remote)?
                .ok_or("Remote download source does not exist")?;
            if entry.is_symlink || entry.is_dir {
                return Err(
                    "Download source must be a regular file; symbolic links are not followed"
                        .into(),
                );
            }
        }
        let (temp, mut output) = LocalTemp::create(local)?;
        let mut remote_times = None;
        let mut metadata_warning = None;
        let result: Result<(), String> = match self.backend()? {
            Backend::Ftp(ftp) => (|| {
                let size = ftp.stream.size(remote).ok().map(|n| n as u64);
                match ftp.stream.mtime(remote) {
                    Ok(mtime) => remote_times = Some((mtime, mtime)),
                    Err(error @ FtpError::UnexpectedResponse(_)) => {
                        metadata_warning = Some(format!(
                            "Remote modification time was unavailable for {remote}: {error}"
                        ))
                    }
                    Err(error) => return Err(err(error)),
                }
                let mut data = ftp.data("RETR", remote)?;
                let copied = copy_cancel(&mut data, &mut output, cancel)?;
                ftp.stream.finalize_retr_stream(data).map_err(err)?;
                if size.is_some_and(|size| size != copied) {
                    return Err("Download size did not match remote file".into());
                }
                Ok(())
            })(),
            Backend::Sftp { sftp, .. } => {
                (|| {
                    let path = sftp_path(remote)?;
                    let stat = sftp.lstat(path).map_err(err)?;
                    if let Some(mtime) = stat.mtime.and_then(|time| i64::try_from(time).ok()) {
                        remote_times = Some((
                            stat.atime
                                .and_then(|time| i64::try_from(time).ok())
                                .unwrap_or(mtime),
                            mtime,
                        ));
                    } else {
                        metadata_warning = Some(format!(
                            "Remote modification time was unavailable for {remote}"
                        ));
                    }
                    if !stat.is_file() {
                        return Err("Download source must be a regular file; symbolic links are not followed".into());
                    }
                    let mut input = sftp.open(path).map_err(err)?;
                    let copied = copy_cancel(&mut input, &mut output, cancel)?;
                    input.close().map_err(err)?;
                    if stat.size.is_some_and(|size| size != copied) {
                        return Err("Download size did not match remote file".into());
                    }
                    Ok(())
                })()
            }
        };
        if result.is_err() && matches!(self.backend, Some(Backend::Ftp(_))) {
            self.invalidate();
        }
        result?;
        if let Some(warning) = metadata_warning {
            self.warnings.push(warning);
        }
        output.sync_all().map_err(err)?;
        drop(output);
        if let Some((atime, mtime)) = remote_times
            && let Err(error) = filetime::set_file_times(
                &temp.0,
                filetime::FileTime::from_unix_time(atime, 0),
                filetime::FileTime::from_unix_time(mtime, 0),
            )
        {
            self.warnings.push(format!(
                "Downloaded bytes verified; timestamps could not be preserved for {}: {error}",
                local.display()
            ));
        }
        check_cancel(cancel)?;
        temp.commit(local)
    }

    pub fn upload(
        &mut self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        validate_text(remote)?;
        check_cancel(cancel)?;
        let meta = fs::symlink_metadata(local).map_err(err)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("Upload source must be a regular file".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Upload source cannot be a reparse point".into());
            }
        }
        let mut input = File::open(local).map_err(err)?;
        let temporary = remote_temp(remote);
        let source_mtime = filetime::FileTime::from_last_modification_time(&meta).unix_seconds();
        let source_atime = filetime::FileTime::from_last_access_time(&meta).unix_seconds();
        let mut metadata_warning = None;
        let result = match self.backend()? {
            Backend::Ftp(ftp) => (|| {
                let mut data = ftp.data("STOR", &temporary)?;
                let copied = copy_cancel(&mut input, &mut data, cancel)?;
                ftp.stream.finalize_put_stream(data).map_err(err)?;
                if copied != meta.len() {
                    return Err("Upload source size changed during transfer".into());
                }
                if let Err(error) = ftp.stream.set_mtime(&temporary, source_mtime) {
                    if !matches!(error, FtpError::UnexpectedResponse(_)) {
                        return Err(err(error));
                    }
                    metadata_warning = Some(format!(
                        "Uploaded bytes verified; modification time could not be preserved for {remote}: {error}"
                    ));
                }
                Ok(())
            })(),
            Backend::Sftp { sftp, .. } => (|| {
                sftp_path(remote)?;
                let mut output = sftp
                    .open_mode(
                        sftp_path(&temporary)?,
                        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
                        0o600,
                        OpenType::File,
                    )
                    .map_err(err)?;
                let copied = copy_cancel(&mut input, &mut output, cancel)?;
                output.close().map_err(err)?;
                if copied != meta.len()
                    || sftp.stat(sftp_path(&temporary)?).map_err(err)?.size != Some(copied)
                {
                    return Err("Upload size verification failed".into());
                }
                match (u64::try_from(source_atime), u64::try_from(source_mtime)) {
                    (Ok(atime), Ok(mtime)) => {
                        if let Err(error) = sftp.setstat(
                            sftp_path(&temporary)?,
                            ssh2::FileStat {
                                size: None,
                                uid: None,
                                gid: None,
                                perm: None,
                                atime: Some(atime),
                                mtime: Some(mtime),
                            },
                        ) {
                            if !matches!(error.code(), ssh2::ErrorCode::SFTP(_)) {
                                return Err(err(error));
                            }
                            metadata_warning = Some(format!(
                                "Uploaded bytes verified; timestamps could not be preserved for {remote}: {error}"
                            ));
                        }
                    }
                    _ => {
                        metadata_warning = Some(format!(
                            "SFTP cannot preserve pre-epoch timestamps for {remote}"
                        ))
                    }
                }
                Ok(())
            })(),
        };
        if let Some(warning) = metadata_warning {
            self.warnings.push(warning);
        }
        if let Err(error) = result {
            if matches!(self.backend, Some(Backend::Ftp(_))) {
                // Terminate an FTP session with unread transfer replies. The random
                // partial upload is retained and its exact path is reported.
                self.invalidate();
                return Err(format!(
                    "{error}; temporary remote file may remain: {temporary}"
                ));
            }
            let cleanup = self.delete(&temporary, false);
            return Err(match cleanup {
                Ok(()) => error,
                Err(e) => format!("{error}; cleanup failed for {temporary}: {e}"),
            });
        }
        if let Err(error) = check_cancel(cancel) {
            let cleanup = self.delete(&temporary, false);
            return Err(match cleanup {
                Ok(()) => error,
                Err(e) => format!("{error}; cleanup failed for {temporary}: {e}"),
            });
        }
        let result = self.commit_upload(&temporary, remote);
        if result.is_err() {
            let _ = self.delete(&temporary, false);
        }
        result
    }

    fn commit_upload(&mut self, temporary: &str, remote: &str) -> Result<(), String> {
        let exists = match self.backend()? {
            Backend::Ftp(ftp) => match ftp.stream.size(remote) {
                Ok(_) => true,
                Err(FtpError::UnexpectedResponse(response))
                    if response.status == Status::FileUnavailable =>
                {
                    false
                }
                Err(e) => return Err(err(e)),
            },
            Backend::Sftp { sftp, .. } => match sftp.lstat(sftp_path(remote)?) {
                Ok(stat) => {
                    if stat.is_dir() {
                        return Err("Upload destination is a directory".into());
                    }
                    true
                }
                Err(e) if matches!(e.code(), ssh2::ErrorCode::SFTP(2 | 10)) => false,
                Err(e) => return Err(err(e)),
            },
        };
        if !exists {
            return self.rename(temporary, remote);
        }
        // SFTP v3 does not guarantee replace-existing rename. Preserve the old
        // object under a sibling backup until the new object has been committed.
        // Once commit begins, finish or roll back even if cancellation arrives.
        let backup = remote_temp(remote);
        self.rename(remote, &backup)?;
        if let Err(commit) = self.rename(temporary, remote) {
            return match self.rename(&backup, remote) {
                Ok(()) => Err(commit),
                Err(restore) => Err(format!(
                    "Upload commit failed: {commit}; restore failed: {restore}; original file retained at {backup}"
                )),
            };
        }
        self.delete(&backup, false).map_err(|e| {
            format!("Upload committed; old backup could not be removed: {backup}: {e}")
        })
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), String> {
        validate_text(path)?;
        match self.backend()? {
            Backend::Ftp(ftp) => ftp.stream.mkdir(path).map_err(err),
            Backend::Sftp { sftp, .. } => sftp.mkdir(sftp_path(path)?, 0o755).map_err(err),
        }
    }

    pub fn rename(&mut self, source: &str, destination: &str) -> Result<(), String> {
        validate_text(source)?;
        validate_text(destination)?;
        match self.backend()? {
            Backend::Ftp(ftp) => ftp.stream.rename(source, destination).map_err(err),
            Backend::Sftp { sftp, .. } => sftp
                .rename(
                    sftp_path(source)?,
                    sftp_path(destination)?,
                    Some(ssh2::RenameFlags::empty()),
                )
                .map_err(err),
        }
    }

    /// Removes one object. Recursive directory removal is deliberately a caller
    /// operation so cancellation and confirmation can be applied per entry.
    /// Server-side rmdir fails for nonempty directories and never follows a link.
    pub fn delete(&mut self, path: &str, is_dir: bool) -> Result<(), String> {
        validate_text(path)?;
        match self.backend()? {
            Backend::Ftp(ftp) => {
                if is_dir {
                    ftp.stream.rmdir(path).map_err(err)
                } else {
                    ftp.stream.rm(path).map_err(err)
                }
            }
            Backend::Sftp { sftp, .. } => {
                let path = sftp_path(path)?;
                let stat = sftp.lstat(path).map_err(err)?;
                if is_dir && stat.is_dir() {
                    sftp.rmdir(path).map_err(err)
                } else {
                    sftp.unlink(path).map_err(err)
                }
            }
        }
    }

    /// Executes a single FTP control command. Data transfers, TLS negotiation,
    /// authentication, and arbitrary SSH shell execution are not command APIs.
    pub fn command(&mut self, command: &str) -> Result<String, String> {
        validate_text(command)?;
        let verb = command
            .split_once(' ')
            .map(|p| p.0)
            .unwrap_or(command)
            .to_ascii_uppercase();
        if ![
            "PWD", "CWD", "CDUP", "NOOP", "SYST", "FEAT", "HELP", "STAT", "SITE", "SIZE", "MDTM",
            "MLST", "MKD", "RMD", "DELE", "RNFR", "RNTO",
        ]
        .contains(&verb.as_str())
        {
            return Err(
                "Unsupported control command; use the transfer operations for data commands".into(),
            );
        }
        match self.backend()? {
            Backend::Ftp(ftp) => {
                let expected: Vec<Status> = (200..400)
                    .map(Status::from)
                    .filter(|code| *code != Status::Unknown)
                    .collect();
                ftp.stream
                    .custom_command(command, &expected)
                    .map_err(err)?
                    .as_string()
                    .map_err(err)
            }
            Backend::Sftp { .. } if verb == "PWD" && command.eq_ignore_ascii_case("PWD") => {
                self.pwd()
            }
            Backend::Sftp { .. } if verb == "NOOP" && command.eq_ignore_ascii_case("NOOP") => {
                Ok("OK".into())
            }
            Backend::Sftp { .. } => {
                Err("SFTP has no FTP control channel; use file operations".into())
            }
        }
    }

    fn invalidate(&mut self) {
        if let Some(Backend::Ftp(ftp)) = self.backend.take() {
            let _ = ftp.stream.get_ref().shutdown(Shutdown::Both);
        }
    }

    pub fn close(&mut self) -> Result<(), String> {
        match self.backend.take() {
            Some(Backend::Ftp(mut ftp)) => ftp.stream.quit().map_err(err),
            Some(Backend::Sftp { mut sftp, session }) => {
                sftp.shutdown().map_err(err)?;
                session.disconnect(None, "NDR closed", None).map_err(err)
            }
            None => Ok(()),
        }
    }
}

impl Ftp {
    fn connect(
        config: &Config,
        stream: TcpStream,
        encoding: &'static encoding_rs::Encoding,
        tls: Option<native_tls::TlsConnector>,
    ) -> Result<Self, String> {
        let ftp = if encoding == encoding_rs::UTF_8 {
            let mut ftp = NativeTlsFtpStream::connect_with_stream(stream).map_err(err)?;
            if let Some(connector) = &tls {
                ftp = ftp
                    .into_secure(NativeTlsConnector::from(connector.clone()), &config.host)
                    .map_err(err)?;
            }
            ftp = ftp.passive_stream_builder(|address| {
                let socket = TcpStream::connect_timeout(&address, TIMEOUT)
                    .map_err(FtpError::ConnectionError)?;
                socket
                    .set_read_timeout(Some(TIMEOUT))
                    .map_err(FtpError::ConnectionError)?;
                socket
                    .set_write_timeout(Some(TIMEOUT))
                    .map_err(FtpError::ConnectionError)?;
                Ok(socket)
            });
            ftp.set_passive_nat_workaround(true);
            if config.passive && ftp.get_ref().peer_addr().map_err(err)?.is_ipv6() {
                ftp.set_mode(suppaftp::Mode::ExtendedPassive);
            }
            if !config.passive {
                ftp = ftp.active_mode(TIMEOUT);
            }
            ftp.login(&config.user, &config.password).map_err(err)?;
            ftp.transfer_type(FileType::Binary).map_err(err)?;
            let _ = ftp.opts("UTF8", Some("ON"));
            FtpControl::Utf8(ftp)
        } else {
            FtpControl::Encoded(EncodedFtp::connect(stream, config, encoding, tls.clone())?)
        };
        Ok(Self {
            stream: ftp,
            passive: config.passive,
            tls,
            host: config.host.clone(),
            encoding,
        })
    }
    // MLSD is allowed to report a symlink-to-directory as type=dir. Recursive
    // operations require POSIX LIST's lstat-style link type. DOS LIST cannot
    // represent that distinction, so it is rejected for recursive operations.
    fn list_nofollow(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        let previous = self.stream.pwd().map_err(err)?;
        validate_text(path)?;
        validate_text(&previous)?;
        self.stream.cwd(path).map_err(err)?;
        let result = (|| {
            let lines = if self.passive {
                self.stream.list(None).map_err(err)?
            } else {
                self.active_listing("LIST")?
            };
            lines
                .into_iter()
                .filter(|line| !line.starts_with("total "))
                .map(|line| {
                    parse_posix(&line).map_err(|error| {
                        format!(
                            "Recursive FTP operation requires POSIX LIST link metadata: {error}"
                        )
                    })
                })
                .collect()
        })();
        let restore = self.stream.cwd(&previous).map_err(err);
        match (result, restore) {
            (Ok(entries), Ok(())) => Ok(entries),
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(format!("Could not restore FTP working directory: {e}")),
        }
    }

    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        let previous = self.stream.pwd().map_err(err)?;
        validate_text(&previous)?;
        self.stream.cwd(path).map_err(err)?;
        let result = (|| {
            let machine = if self.passive {
                self.stream.mlsd(None).map_err(err)
            } else {
                self.active_listing("MLSD")
            };
            match machine {
                Ok(lines) => lines
                    .iter()
                    .filter_map(|line| match parse_mlsd(line) {
                        Ok(Some(e)) => Some(Ok(e)),
                        Ok(None) => None,
                        Err(e) => Some(Err(e)),
                    })
                    .collect(),
                Err(error)
                    if error.contains("500") || error.contains("502") || error.contains("504") =>
                {
                    let lines = if self.passive {
                        self.stream.list(None).map_err(err)?
                    } else {
                        self.active_listing("LIST")?
                    };
                    lines
                        .into_iter()
                        .filter(|line| !line.starts_with("total "))
                        .map(|line| {
                            if let Ok(entry) = parse_posix(&line) {
                                return Ok(entry);
                            }
                            let file = suppaftp::list::File::from_dos_line(&line).map_err(err)?;
                            validate_name(file.name())?;
                            Ok(RemoteEntry {
                                name: file.name().to_owned(),
                                is_dir: file.is_directory() && !file.is_symlink(),
                                size: file.size() as u64,
                                is_symlink: file.is_symlink(),
                                modified: file
                                    .modified()
                                    .duration_since(UNIX_EPOCH)
                                    .ok()
                                    .and_then(|d| i64::try_from(d.as_secs()).ok()),
                                mode: None,
                                permission: None,
                            })
                        })
                        .collect()
                }
                Err(e) => Err(e),
            }
        })();
        let restore = self.stream.cwd(&previous).map_err(err);
        match (result, restore) {
            (Ok(entries), Ok(())) => Ok(entries),
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(format!("Could not restore FTP working directory: {e}")),
        }
    }

    // suppaftp does not set timeouts on accepted active-listing sockets. Accept
    // and read these explicitly so active FTP/FTPS listings are bounded as well.
    fn data(&mut self, verb: &str, path: &str) -> Result<Box<dyn TransferStream>, String> {
        if self.passive {
            self.stream.data(verb, path)
        } else {
            self.active_data(&format!("{verb} {path}"))
        }
    }

    fn active_data(&mut self, command: &str) -> Result<Box<dyn TransferStream>, String> {
        let control = self.stream.get_ref();
        let listener =
            TcpListener::bind((control.local_addr().map_err(err)?.ip(), 0)).map_err(err)?;
        listener.set_nonblocking(true).map_err(err)?;
        let peer = control.peer_addr().map_err(err)?.ip();
        let address = listener.local_addr().map_err(err)?;
        if let Err(error) = self.stream.eprt(address) {
            if let (std::net::IpAddr::V4(ip), FtpError::UnexpectedResponse(response)) =
                (address.ip(), &error)
            {
                if matches!(response.status as u32, 500 | 502 | 504 | 522) {
                    let [a, b, c, d] = ip.octets();
                    let port = address.port();
                    self.stream
                        .custom_command(
                            &format!("PORT {a},{b},{c},{d},{},{}", port / 256, port % 256),
                            &[Status::CommandOk],
                        )
                        .map_err(err)?;
                } else {
                    return Err(err(error));
                }
            } else {
                return Err(err(error));
            }
        }
        self.stream
            .custom_command(command, &[Status::AboutToSend, Status::AlreadyOpen])
            .map_err(err)?;
        let deadline = Instant::now() + TIMEOUT;
        let data = loop {
            match listener.accept() {
                Ok((stream, address)) if address.ip() == peer => break stream,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(err(e)),
            }
            if Instant::now() >= deadline {
                return Err("Active FTP data accept timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        set_timeout(&data)?;
        if let Some(connector) = &self.tls {
            let secure = connector.connect(&self.host, data).map_err(err)?;
            Ok(Box::new(Wire::Secure(Box::new(secure))))
        } else {
            Ok(Box::new(Wire::Plain(data)))
        }
    }

    fn active_listing(&mut self, command: &str) -> Result<Vec<String>, String> {
        let mut data = self.active_data(command)?;
        let mut bytes = Vec::new();
        data.read_to_end(&mut bytes).map_err(err)?;
        drop(data);
        self.stream
            .finalize_retr_stream(std::io::empty())
            .map_err(err)?;
        let text = self
            .encoding
            .decode_without_bom_handling_and_without_replacement(&bytes)
            .ok_or("FTP listing cannot be decoded exactly; refusing ambiguous filenames")?;
        Ok(text
            .split_terminator('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
            .filter(|line| !line.is_empty())
            .collect())
    }
}

fn parse_mlsd(line: &str) -> Result<Option<RemoteEntry>, String> {
    let (facts, name) = line.split_once(' ').ok_or("Invalid MLSD line")?;
    let mut kind = None;
    let mut size = 0;
    let mut modified = None;
    let mut mode = None;
    let mut permission = None;
    for fact in facts.split(';') {
        if let Some((key, value)) = fact.split_once('=') {
            if key.eq_ignore_ascii_case("type") {
                kind = Some(value.to_ascii_lowercase());
            }
            if key.eq_ignore_ascii_case("size") {
                size = value.parse::<u64>().map_err(err)?;
            }
            if key.eq_ignore_ascii_case("modify") {
                modified = parse_mlsd_time(value);
            }
            if key.eq_ignore_ascii_case("unix.mode") {
                mode = u32::from_str_radix(value, 8).ok().map(|m| m & 0o7777);
            }
            if key.eq_ignore_ascii_case("perm") {
                permission = Some(value.to_owned());
            }
        }
    }
    match kind.as_deref() {
        Some("cdir" | "pdir") => Ok(None),
        Some(kind) => {
            validate_name(name)?;
            Ok(Some(RemoteEntry {
                name: name.to_owned(),
                is_dir: kind == "dir",
                size,
                is_symlink: kind == "link" || kind.contains("slink"),
                modified,
                mode,
                permission,
            }))
        }
        None => Err("MLSD entry has no type".into()),
    }
}

fn parse_posix(line: &str) -> Result<RemoteEntry, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    parse_posix_at(line, now)
}

/// One `ls -l` line. A time of "Mon DD HH:MM" is this year (relative to now); if that is more than a day in the future, last year.
fn parse_posix_at(line: &str, now: i64) -> Result<RemoteEntry, String> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| regex::Regex::new(r"^([dl-])([rwxstST-]{9})[+@.]?\s+\d+\s+\S+\s+\S+\s+(\d+)\s+(\S+)\s+(\d+)\s+(\d{2}:\d{2}|\d{4}) (.*)$").unwrap());
    let caps = pattern
        .captures(line)
        .ok_or("Unsupported or ambiguous POSIX FTP listing")?;
    let link = &caps[1] == "l";
    let name = if link {
        if caps[7].matches(" -> ").count() != 1 {
            return Err("Ambiguous FTP symbolic-link name".into());
        }
        caps[7].split_once(" -> ").unwrap().0
    } else {
        &caps[7]
    };
    validate_name(name)?;
    Ok(RemoteEntry {
        name: name.to_owned(),
        is_dir: &caps[1] == "d",
        size: caps[3].parse().map_err(err)?,
        is_symlink: link,
        modified: posix_listing_time(&caps[4], &caps[5], &caps[6], now),
        mode: Some(posix_mode(&caps[2])),
        permission: Some(format!("{}{}", &caps[1], &caps[2])),
    })
}

/// "rwxr-sr-T" -> permission bits (s/S = setuid/setgid, t/T = sticky, lower case includes execute).
fn posix_mode(text: &str) -> u32 {
    let b = text.as_bytes();
    let mut mode = 0;
    for (group, special) in [(0usize, 0o4000), (1, 0o2000), (2, 0o1000)] {
        let shift = 6 - group as u32 * 3;
        if b[group * 3] == b'r' {
            mode |= 4 << shift;
        }
        if b[group * 3 + 1] == b'w' {
            mode |= 2 << shift;
        }
        match b[group * 3 + 2] {
            b'x' => mode |= 1 << shift,
            b's' | b't' => mode |= (1 << shift) | special,
            b'S' | b'T' => mode |= special,
            _ => {}
        }
    }
    mode
}

/// Date (Gregorian, UTC) -> Unix seconds.
fn unix_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Year of Unix seconds (UTC).
fn year_of(seconds: i64) -> i64 {
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}

/// MLSD modify=YYYYMMDDHHMMSS[.sss](UTC).
fn parse_mlsd_time(value: &str) -> Option<i64> {
    let digits = value.split('.').next()?;
    if digits.len() != 14 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |range: std::ops::Range<usize>| digits[range].parse::<i64>().ok();
    unix_seconds(n(0..4)?, n(4..6)?, n(6..8)?, n(8..10)?, n(10..12)?, n(12..14)?)
}

fn posix_listing_time(month: &str, day: &str, clock_or_year: &str, now: i64) -> Option<i64> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month))? as i64 + 1;
    let day = day.parse::<i64>().ok()?;
    if let Some((hour, minute)) = clock_or_year.split_once(':') {
        let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
        let year = year_of(now);
        let this_year = unix_seconds(year, month, day, hour, minute, 0)?;
        return if this_year > now + 86_400 { unix_seconds(year - 1, month, day, hour, minute, 0) } else { Some(this_year) };
    }
    unix_seconds(clock_or_year.parse().ok()?, month, day, 0, 0, 0)
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    #[test]
    fn listings_carry_times_and_permission_modes() {
        let entry = parse_mlsd("type=file;size=12;modify=20240229123456.789;UNIX.mode=0754; data.bin").unwrap().unwrap();
        assert_eq!((entry.name.as_str(), entry.size, entry.modified, entry.mode), ("data.bin", 12, Some(1_709_210_096), Some(0o754)));
        assert_eq!(parse_mlsd("type=dir; folder").unwrap().unwrap().modified, None);
        // The listing's own permission text (original attribute cell on FTP panes): MLSD "perm" fact, LIST mode column.
        assert_eq!(parse_mlsd("type=dir;perm=eldfmcp; pub").unwrap().unwrap().permission.as_deref(), Some("eldfmcp"));
        assert_eq!(parse_mlsd("type=file;size=1; plain").unwrap().unwrap().permission, None);
        // 2024-06-01 00:00:00 UTC
        let now = unix_seconds(2024, 6, 1, 0, 0, 0).unwrap();
        let file = parse_posix_at("-rwsr-xr-T 1 user group 42 Mar 05 13:07 report.txt", now).unwrap();
        assert_eq!((file.mode, file.modified), (Some(0o5754), unix_seconds(2024, 3, 5, 13, 7, 0)));
        assert_eq!(file.permission.as_deref(), Some("-rwsr-xr-T"));
        let older = parse_posix_at("drwxr-x--- 2 user group 4096 Dec 24 08:00 archive", now).unwrap();
        assert_eq!((older.is_dir, older.mode, older.modified), (true, Some(0o750), unix_seconds(2023, 12, 24, 8, 0, 0)), "a future clock date is last year");
        let dated = parse_posix_at("-rw-r--r-- 1 u g 0 Jan 02 2001 old.txt", now).unwrap();
        assert_eq!(dated.modified, unix_seconds(2001, 1, 2, 0, 0, 0));
        assert_eq!(posix_mode("rwxrwsrwt"), 0o3777);
        assert_eq!(year_of(now), 2024);
        assert_eq!(year_of(unix_seconds(2023, 12, 31, 23, 59, 59).unwrap()), 2023);
        assert_eq!(parse_mlsd_time("2024022912345"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    /// Original FUN_004404c0: received lines as is, sent lines "current remote folder > command", "PASS " masked with '*' from the sixth character, last line selected.
    #[test]
    fn protocol_log_follows_the_original_ftp_log() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(ProtocolLog::default()));
        with_protocol_log(Some(&log), || {
            log::trace!(target: "suppaftp::sync_ftp", "CC IN: {:?}", b"220 Ready\r\n".to_vec());
            log::trace!(target: "suppaftp::sync_ftp", "Code parsed from response: Ready (220)");
            log::trace!(target: "suppaftp::sync_ftp", "CC IN: {:?}", b"220 Ready\r\n".to_vec());
            log::trace!(target: "suppaftp::sync_ftp", "CC OUT: USER bob");
            log.lock().unwrap().prefix = "/pub".into();
            log::trace!(target: "suppaftp::sync_ftp", "CC OUT: PASS secret");
            log::trace!(target: "other", "CC OUT: NOT FTP");
        });
        log::trace!(target: "suppaftp::sync_ftp", "CC OUT: OUTSIDE");
        let log = log.lock().unwrap();
        assert_eq!(log.lines, ["220 Ready", " > USER bob", "/pub > PASS ******"]);
        assert_eq!(log.selected, Some(2));
    }

    struct Fixture {
        child: Child,
        root: PathBuf,
        port: u16,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn start(protocol: &str) -> Self {
            let root = std::env::temp_dir().join(temp_name());
            fs::create_dir(&root).unwrap();
            let mut child = Command::new("python")
                .args(["-u", "-c", PYTHON_FIXTURE, protocol])
                .arg(&root)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("Python fixture is required for this ignored integration test");
            let mut ready = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            let port = ready
                .trim()
                .parse()
                .expect("Python fixture failed to start");
            Self { child, root, port }
        }
        fn config(&self, protocol: &str) -> Config {
            Config {
                protocol: if protocol.starts_with("sftp") {
                    "sftp".into()
                } else {
                    protocol.trim_end_matches("-euc-kr").into()
                },
                encoding_name: if protocol.ends_with("-euc-kr") {
                    "EUC-KR".into()
                } else {
                    "UTF-8".into()
                },
                host: "127.0.0.1".into(),
                port: self.port,
                user: "fixture".into(),
                password: "owned-test-only".into(),
                passive: true,
                ..Config::default()
            }
        }
    }

    // Entirely owned localhost servers and throwaway keys. No user configuration,
    // credentials, trust store, external address, or production server is used.
    const PYTHON_FIXTURE: &str = r#"
import os, sys, socket, threading, time, logging, stat
logging.disable(logging.CRITICAL)
protocol, root = sys.argv[1:]
if protocol in ('ftp', 'ftps', 'ftp-euc-kr', 'ftps-euc-kr'):
    from pyftpdlib.authorizers import DummyAuthorizer
    from pyftpdlib.handlers import FTPHandler, TLS_FTPHandler
    from pyftpdlib.servers import FTPServer
    if os.environ.get('NDR_FIXTURE_TRACE'):
        from pyftpdlib.log import config_logging
        logging.disable(logging.NOTSET); config_logging(level=logging.DEBUG)
    handler = FTPHandler
    if protocol.startswith('ftps'):
        import datetime, ipaddress
        from cryptography import x509
        from cryptography.x509.oid import NameOID
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import rsa
        key = rsa.generate_private_key(public_exponent=65537,key_size=2048)
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'localhost')])
        cert = x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(datetime.datetime.now(datetime.timezone.utc)-datetime.timedelta(days=1)).not_valid_after(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(days=1)).add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost')]),False).sign(key,hashes.SHA256())
        pem = os.path.join(root,'fixture.pem')
        open(pem,'wb').write(key.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.TraditionalOpenSSL,serialization.NoEncryption()) + cert.public_bytes(serialization.Encoding.PEM))
        open(os.path.join(root,'fixture-cert.pem'),'wb').write(cert.public_bytes(serialization.Encoding.PEM))
        handler = TLS_FTPHandler
        handler.certfile = pem
        handler.tls_control_required = True
        handler.tls_data_required = True
    if protocol.endswith('-euc-kr'): handler.encoding = 'cp949'
    auth = DummyAuthorizer()
    auth.add_user('fixture','owned-test-only',root,perm='elradfmwMT')
    handler.authorizer = auth
    from pyftpdlib.filesystems import AbstractedFS
    class LinkFS(AbstractedFS):
        def lstat(self,p):
            result = super().lstat(p)
            if os.path.basename(p) == 'virtual-link':
                values=list(result); values[0]=stat.S_IFLNK | 0o777; return os.stat_result(values)
            return result
        def readlink(self,p):
            if os.path.basename(p) == 'virtual-link': return '/link-target-sentinel'
            return super().readlink(p)
    handler.abstracted_fs = LinkFS
    base_handler = handler
    class Handler(base_handler):
        def ftp_MFMT(self,p,timeval):
            if os.path.exists(os.path.join(root,'reject-metadata')):
                self.respond('502 Timestamp updates are unsupported.'); return
            return super().ftp_MFMT(p,timeval)
        def ftp_RNTO(self,p):
            marker = os.path.join(root,'inject-rename-failure')
            if os.path.basename(p) == 'remote-tree' and os.path.exists(marker):
                os.unlink(marker); self._rnfr=None; self.respond('550 Injected commit failure.'); return
            return super().ftp_RNTO(p)
    handler = Handler
    server = FTPServer(('127.0.0.1',0),handler)
    print(server.socket.getsockname()[1],flush=True)
    server.serve_forever(timeout=0.05,blocking=True,handle_exit=False)
else:
    import paramiko
    import base64, hashlib
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ed25519, ec
    from paramiko import SFTPAttributes, SFTPHandle, SFTP_OK
    if protocol == 'sftp-ed25519':
        raw_key=ed25519.Ed25519PrivateKey.generate()
        key_path=os.path.join(root,'host-ed25519-key')
        open(key_path,'wb').write(raw_key.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.OpenSSH,serialization.NoEncryption()))
        key=paramiko.Ed25519Key.from_private_key_file(key_path)
    else: key = paramiko.RSAKey.generate(2048)
    open(os.path.join(root,'host-fingerprint.txt'),'w').write('SHA256:'+base64.b64encode(hashlib.sha256(key.asbytes()).digest()).decode().rstrip('='))
    allowed_keys=[]
    if protocol == 'sftp-ed25519':
        raw_user=ed25519.Ed25519PrivateKey.generate()
        user_path=os.path.join(root,'user-ed25519-key')
        open(user_path,'wb').write(raw_user.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.OpenSSH,serialization.NoEncryption()))
        allowed_keys.append(paramiko.Ed25519Key.from_private_key_file(user_path).get_base64())
        raw_user=ec.generate_private_key(ec.SECP256R1())
        for name,fmt in [('user-ecdsa-encrypted-pem',serialization.PrivateFormat.TraditionalOpenSSL),('user-ecdsa-encrypted-openssh',serialization.PrivateFormat.OpenSSH)]:
            user_path=os.path.join(root,name)
            open(user_path,'wb').write(raw_user.private_bytes(serialization.Encoding.PEM,fmt,serialization.BestAvailableEncryption(b'owned-key-passphrase')))
            allowed_keys.append(paramiko.ECDSAKey.from_private_key_file(user_path,password='owned-key-passphrase').get_base64())
    class SSH(paramiko.ServerInterface):
        def check_auth_password(self,u,p):
            with open(os.path.join(root,'auth.log'),'ab') as f: f.write(b'password-auth\n')
            return paramiko.AUTH_SUCCESSFUL if (u,p)==('fixture','owned-test-only') else paramiko.AUTH_FAILED
        def check_auth_publickey(self,u,k):
            with open(os.path.join(root,'auth.log'),'ab') as f: f.write(b'public-key-auth\n')
            return paramiko.AUTH_SUCCESSFUL if u=='fixture' and k.get_base64() in allowed_keys else paramiko.AUTH_FAILED
        def get_allowed_auths(self,u): return 'password,publickey'
        def check_channel_request(self,k,i): return paramiko.OPEN_SUCCEEDED if k=='session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    class SFTP(paramiko.SFTPServerInterface):
        def path(self,p):
            value = os.path.abspath(os.path.join(root,p.lstrip('/')))
            if os.path.commonpath((value,root)) != root: raise OSError('outside fixture')
            return value
        def canonicalize(self,p): return '/' + os.path.relpath(self.path(p),root).replace('\\','/').replace('.','',1) if p=='.' else p
        def list_folder(self,p):
            try:
                out=[]
                for name in os.listdir(self.path(p)):
                    attr = SFTPAttributes.from_stat(os.lstat(os.path.join(self.path(p),name)))
                    if name == 'virtual-link': attr.st_mode = stat.S_IFLNK | 0o777
                    attr.filename=name
                    out.append(attr)
                return out
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def stat(self,p):
            try: return SFTPAttributes.from_stat(os.stat(self.path(p)))
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def lstat(self,p):
            try:
                attr = SFTPAttributes.from_stat(os.lstat(self.path(p)))
                if os.path.basename(p) == 'virtual-link': attr.st_mode = stat.S_IFLNK | 0o777
                return attr
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def chattr(self,p,attr):
            try:
                if attr.st_mtime is not None and os.path.exists(os.path.join(root,'reject-metadata')): return paramiko.SFTP_OP_UNSUPPORTED
                if attr.st_mode is not None:
                    with open(os.path.join(root,'chmod.log'),'w') as f: f.write(str(attr.st_mode))
                    os.chmod(self.path(p),attr.st_mode)
                if attr.st_mtime is not None:
                    os.utime(self.path(p),(attr.st_atime if attr.st_atime is not None else attr.st_mtime,attr.st_mtime))
                return SFTP_OK
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def open(self,p,flags,attr):
            try:
                if os.path.basename(p) == 'virtual-link': open(os.path.join(root,'link-followed'),'wb').close()
                fd=os.open(self.path(p),flags | getattr(os,'O_BINARY',0),0o600)
                f=os.fdopen(fd,'r+b' if flags & os.O_RDWR else 'wb' if flags & os.O_WRONLY else 'rb')
                class Handle(SFTPHandle):
                    def read(self,offset,length):
                        if os.path.basename(p) == 'fail-read.bin' and offset >= 65536: return paramiko.SFTP_FAILURE
                        if os.path.basename(p) == 'slow-read.bin':
                            open(os.path.join(root,'read-started'),'wb').close(); time.sleep(0.1)
                        return super().read(offset,length)
                    def write(self,offset,data):
                        if os.path.exists(os.path.join(root,'inject-write-failure')) and offset >= 65536: return paramiko.SFTP_FAILURE
                        return super().write(offset,data)
                h=Handle(flags); h.readfile=f; h.writefile=f
                return h
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def remove(self,p):
            try: os.unlink(self.path(p)); return SFTP_OK
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def rename(self,a,b):
            try:
                marker=os.path.join(root,'inject-rename-failure')
                if os.path.basename(b) == 'remote-tree' and os.path.exists(marker):
                    os.unlink(marker); return paramiko.SFTP_FAILURE
                if os.path.exists(self.path(b)): return paramiko.SFTP_FAILURE
                os.rename(self.path(a),self.path(b)); return SFTP_OK
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def mkdir(self,p,attr):
            try: os.mkdir(self.path(p)); return SFTP_OK
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
        def rmdir(self,p):
            try: os.rmdir(self.path(p)); return SFTP_OK
            except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
    def serve(sock):
        transport=paramiko.Transport(sock)
        transport.get_security_options().kex = ('diffie-hellman-group14-sha256',)
        transport.add_server_key(key)
        transport.set_subsystem_handler('sftp',paramiko.SFTPServer,SFTP)
        try:
            transport.start_server(server=SSH())
            while transport.is_active(): time.sleep(0.02)
        except Exception as e: print('SFTP fixture:',repr(e),file=sys.stderr,flush=True)
        finally: transport.close()
    listener=socket.socket()
    listener.bind(('127.0.0.1',0)); listener.listen(5)
    print(listener.getsockname()[1],flush=True)
    while True:
        sock,_=listener.accept()
        threading.Thread(target=serve,args=(sock,),daemon=True).start()
"#;

    #[test]
    fn protocol_injection_is_rejected_without_rewriting_unicode_paths() {
        for value in ["/a\r\nDELE /b", "a\0b", "a\tb", "a\u{7f}b"] {
            assert!(validate_text(value).is_err(), "accepted {value:?}");
        }
        for value in ["/한글/공백 이름.txt", "/a\\b//../literal", " file "] {
            assert!(validate_text(value).is_ok());
        }
    }

    #[test]
    fn sha256_fingerprints_normalize_padding_and_reject_other_hashes() {
        let digest = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            normalize_fingerprint(&format!(" SHA256:{digest}= ")).unwrap(),
            format!("SHA256:{digest}")
        );
        for value in [
            "",
            "MD5:11:22",
            "SHA256:a",
            "SHA256:!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!",
        ] {
            assert!(normalize_fingerprint(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn mlsd_preserves_leading_trailing_spaces_and_link_boundaries() {
        let entry = parse_mlsd("type=file;size=7;  한글 이름 ")
            .unwrap()
            .unwrap();
        assert_eq!(entry.name, " 한글 이름 ");
        assert_eq!(entry.size, 7);
        assert!(
            !parse_mlsd("type=OS.unix=slink:/target;size=3; link")
                .unwrap()
                .unwrap()
                .is_dir
        );
        assert!(parse_mlsd("type=pdir; ..").unwrap().is_none());
        assert!(parse_mlsd("type=file; /escape").is_err());
    }

    #[test]
    fn atomic_local_cancel_and_failure_preserve_existing_file() {
        let directory = std::env::temp_dir().join(temp_name());
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("한글.txt");
        fs::write(&destination, b"old content").unwrap();
        {
            let (temp, mut output) = LocalTemp::create(&destination).unwrap();
            let cancelled = AtomicBool::new(true);
            assert!(copy_cancel(&mut &b"new content"[..], &mut output, &cancelled).is_err());
            drop(output);
            drop(temp);
        }
        assert_eq!(fs::read(&destination).unwrap(), b"old content");
        let (temp, mut output) = LocalTemp::create(&destination).unwrap();
        output.write_all(b"new content").unwrap();
        output.sync_all().unwrap();
        drop(output);
        temp.commit(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new content");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn tree_local_names_cannot_escape_or_alias_windows_paths() {
        for name in [
            "..",
            ".",
            "/absolute",
            "a/b",
            "a\\b",
            "C:relative",
            "CON",
            "nul.txt",
            "LPT1.foo",
            "tail ",
            "tail.",
            "x?y",
        ] {
            assert!(validate_local_name(name).is_err(), "accepted {name:?}");
        }
        assert!(validate_local_name(" 한글 이름.txt").is_ok());
    }

    #[test]
    #[ignore = "owned localhost integration; requires Python, pyftpdlib, paramiko, cryptography and pyOpenSSL"]
    fn owned_protocol_fixture() {
        for protocol in ["ftp", "ftp-euc-kr", "sftp"] {
            let fixture = Fixture::start(protocol);
            let mut config = fixture.config(protocol);
            if protocol == "sftp" {
                let error = Client::connect(&config)
                    .err()
                    .expect("unknown SSH host must be rejected");
                let error: serde_json::Value = serde_json::from_str(&error)
                    .unwrap_or_else(|parse| panic!("Expected host-key JSON, got {error}; {parse}"));
                assert_eq!(error["kind"], "unknown_host_key");
                assert!(
                    !fixture.root.join("auth.log").exists(),
                    "authentication attempted before host verification"
                );
                config.fingerprint = error["candidate"].as_str().unwrap().into();
                let mut wrong = config.clone();
                wrong.fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode([0u8; 32]));
                let error = Client::connect(&wrong)
                    .err()
                    .expect("changed SSH host must be rejected");
                assert!(error.contains("host_key_mismatch"));
                assert!(!fixture.root.join("auth.log").exists());
            }
            for passive in if protocol != "sftp" {
                vec![true, false]
            } else {
                vec![true]
            } {
                eprintln!("Protocol fixture {protocol}, passive={passive}");
                config.passive = passive;
                let mut client = Client::connect(&config).unwrap();
                assert!(client.pwd().unwrap().starts_with('/'));
                assert!(client.command("PWD\r\nDELE /oops").is_err());
                assert!(client.mkdir("/bad\nname").is_err());
                // Fixture storage is Windows, which strips trailing spaces.
                // Exact trailing-space parsing is covered separately above.
                let name = " 한글 이름.txt";
                let remote = format!("/{name}");
                let source = fixture.root.join("local-source");
                let destination = fixture.root.join("local-destination");
                fs::write(&source, b"first version\0binary").unwrap();
                filetime::set_file_mtime(
                    &source,
                    filetime::FileTime::from_unix_time(1_600_000_000, 0),
                )
                .unwrap();
                let cancel = AtomicBool::new(false);
                client.upload(&source, &remote, &cancel).unwrap();
                assert_eq!(
                    filetime::FileTime::from_last_modification_time(
                        &fs::metadata(fixture.root.join(name)).unwrap()
                    )
                    .unix_seconds(),
                    1_600_000_000,
                    "upload must preserve modification time"
                );
                assert!(client.chmod(&remote, 0o10000).is_err());
                assert!(client.chmod("bad\r\npath", 0o644).is_err());
                client.chmod(&remote, 0o600).unwrap();
                if protocol == "sftp" {
                    assert_eq!(
                        fs::read_to_string(fixture.root.join("chmod.log")).unwrap(),
                        "384"
                    );
                    assert!(client.permissions(&remote).unwrap().is_some());
                }
                assert!(
                    client
                        .list("/")
                        .unwrap()
                        .iter()
                        .any(|e| e.name == name && !e.is_dir && e.size == 20)
                );
                fs::write(&source, b"second version").unwrap();
                filetime::set_file_mtime(
                    &source,
                    filetime::FileTime::from_unix_time(1_600_000_120, 0),
                )
                .unwrap();
                client.upload(&source, &remote, &cancel).unwrap();
                fs::write(&destination, b"old local").unwrap();
                cancel.store(true, Ordering::Relaxed);
                assert!(client.download(&remote, &destination, &cancel).is_err());
                assert_eq!(fs::read(&destination).unwrap(), b"old local");
                assert!(client.upload(&source, &remote, &cancel).is_err());
                assert_eq!(
                    fs::read(fixture.root.join(name)).unwrap(),
                    b"second version"
                );
                cancel.store(false, Ordering::Relaxed);
                assert!(
                    client
                        .download("/missing-source", &destination, &cancel)
                        .is_err()
                );
                assert_eq!(fs::read(&destination).unwrap(), b"old local");
                // A missing path is rejected before opening a data channel, so
                // this healthy session remains usable without reconnecting.
                assert!(client.command("NOOP").is_ok());
                client.download(&remote, &destination, &cancel).unwrap();
                assert_eq!(fs::read(&destination).unwrap(), b"second version");
                assert_eq!(
                    filetime::FileTime::from_last_modification_time(
                        &fs::metadata(&destination).unwrap()
                    )
                    .unix_seconds(),
                    1_600_000_120,
                    "download must preserve modification time"
                );
                client.rename(&remote, "/renamed").unwrap();
                client.delete("/renamed", false).unwrap();
                client.mkdir("/empty").unwrap();
                client.delete("/empty", true).unwrap();
                let tree = fixture.root.join("source-tree");
                fs::create_dir_all(tree.join("하위 폴더")).unwrap();
                fs::write(tree.join("하위 폴더").join("자료.txt"), b"nested content").unwrap();
                if protocol == "sftp" {
                    filetime::set_file_mtime(
                        &tree,
                        filetime::FileTime::from_unix_time(1_600_000_300, 0),
                    )
                    .unwrap();
                    filetime::set_file_mtime(
                        &tree.join("하위 폴더"),
                        filetime::FileTime::from_unix_time(1_600_000_400, 0),
                    )
                    .unwrap();
                }
                let output_tree = fixture.root.join("downloaded-tree");
                eprintln!("Tree fixture {protocol}, passive={passive}");
                client.upload_tree(&tree, "/remote-tree", &cancel).unwrap();
                if protocol == "sftp" {
                    assert_eq!(
                        filetime::FileTime::from_last_modification_time(
                            &fs::metadata(fixture.root.join("remote-tree")).unwrap()
                        )
                        .unix_seconds(),
                        1_600_000_300,
                        "SFTP tree upload must preserve directory time"
                    );
                }
                client
                    .download_tree("/remote-tree", &output_tree, &cancel)
                    .unwrap();
                if protocol == "sftp" {
                    assert_eq!(
                        filetime::FileTime::from_last_modification_time(
                            &fs::metadata(&output_tree).unwrap()
                        )
                        .unix_seconds(),
                        1_600_000_300,
                        "SFTP tree download must preserve directory time"
                    );
                    assert_eq!(
                        filetime::FileTime::from_last_modification_time(
                            &fs::metadata(output_tree.join("하위 폴더")).unwrap()
                        )
                        .unix_seconds(),
                        1_600_000_400
                    );
                }
                assert_eq!(
                    fs::read(output_tree.join("하위 폴더").join("자료.txt")).unwrap(),
                    b"nested content"
                );
                assert!(
                    client
                        .download_tree("/remote-tree", &output_tree, &cancel)
                        .is_err(),
                    "existing tree must not be overwritten"
                );
                fs::write(output_tree.join("local-extra"), b"keep local extra").unwrap();
                fs::write(
                    output_tree.join("하위 폴더").join("자료.txt"),
                    b"stale local",
                )
                .unwrap();
                client
                    .download_tree_with_conflicts(
                        "/remote-tree",
                        &output_tree,
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::Overwrite,
                        &mut |_| {},
                    )
                    .unwrap();
                assert!(output_tree.join("local-extra").exists());
                assert_eq!(
                    fs::read(output_tree.join("하위 폴더").join("자료.txt")).unwrap(),
                    b"nested content"
                );
                client
                    .download_tree_with_conflicts(
                        "/remote-tree",
                        &output_tree,
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::ReplaceFolder,
                        &mut |_| {},
                    )
                    .unwrap();
                assert!(!output_tree.join("local-extra").exists());
                assert!(
                    client
                        .download_tree_with_conflicts(
                            "/remote-tree",
                            &output_tree,
                            &cancel,
                            &mut |_, _, _| crate::fsops::ConflictChoice::Cancel,
                            &mut |_| {}
                        )
                        .is_err()
                );
                fs::write(
                    fixture.root.join("remote-tree").join("remote-extra"),
                    b"keep remote extra",
                )
                .unwrap();
                fs::write(tree.join("하위 폴더").join("자료.txt"), b"updated nested").unwrap();
                client
                    .upload_tree_with_conflicts(
                        &tree,
                        "/remote-tree",
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::Overwrite,
                        &mut |_| {},
                    )
                    .unwrap();
                assert!(
                    fixture
                        .root
                        .join("remote-tree")
                        .join("remote-extra")
                        .exists()
                );
                assert_eq!(
                    fs::read(
                        fixture
                            .root
                            .join("remote-tree")
                            .join("하위 폴더")
                            .join("자료.txt")
                    )
                    .unwrap(),
                    b"updated nested"
                );
                fs::write(
                    tree.join("하위 폴더").join("자료.txt"),
                    b"replacement must roll back",
                )
                .unwrap();
                fs::write(fixture.root.join("inject-rename-failure"), b"true").unwrap();
                assert!(
                    client
                        .upload_tree_with_conflicts(
                            &tree,
                            "/remote-tree",
                            &cancel,
                            &mut |_, _, _| crate::fsops::ConflictChoice::ReplaceFolder,
                            &mut |_| {}
                        )
                        .is_err()
                );
                assert_eq!(
                    fs::read(
                        fixture
                            .root
                            .join("remote-tree")
                            .join("하위 폴더")
                            .join("자료.txt")
                    )
                    .unwrap(),
                    b"updated nested"
                );
                client
                    .upload_tree_with_conflicts(
                        &tree,
                        "/remote-tree",
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::Skip,
                        &mut |_| {},
                    )
                    .unwrap();
                assert_eq!(
                    fs::read(
                        fixture
                            .root
                            .join("remote-tree")
                            .join("하위 폴더")
                            .join("자료.txt")
                    )
                    .unwrap(),
                    b"updated nested"
                );
                client
                    .upload_tree_with_conflicts(
                        &tree,
                        "/remote-tree",
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::Rename,
                        &mut |_| {},
                    )
                    .unwrap();
                assert!(fixture.root.join("remote-tree (1)").exists());
                client.delete_tree("/remote-tree (1)", &cancel).unwrap();
                client
                    .download_tree_with_conflicts(
                        "/remote-tree",
                        &output_tree,
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::Rename,
                        &mut |_| {},
                    )
                    .unwrap();
                let renamed_tree = fixture.root.join("downloaded-tree (1)");
                assert!(renamed_tree.exists());
                fs::remove_dir_all(renamed_tree).unwrap();
                client
                    .upload_tree_with_conflicts(
                        &tree,
                        "/remote-tree",
                        &cancel,
                        &mut |_, _, _| crate::fsops::ConflictChoice::ReplaceFolder,
                        &mut |_| {},
                    )
                    .unwrap();
                assert!(
                    !fixture
                        .root
                        .join("remote-tree")
                        .join("remote-extra")
                        .exists()
                );
                client.delete_tree("/remote-tree", &cancel).unwrap();
                fs::write(tree.join("virtual-link"), b"link metadata fixture").unwrap();
                fs::write(fixture.root.join("link-target-sentinel"), b"do not touch").unwrap();
                client.upload_tree(&tree, "/link-tree", &cancel).unwrap();
                let refused = fixture.root.join("refused-tree");
                assert!(
                    client
                        .download_tree("/link-tree", &refused, &cancel)
                        .is_err()
                );
                assert!(!refused.exists());
                client.delete_tree("/link-tree", &cancel).unwrap();
                assert_eq!(
                    fs::read(fixture.root.join("link-target-sentinel")).unwrap(),
                    b"do not touch"
                );
                assert!(!fixture.root.join("link-followed").exists());
                fs::remove_dir_all(tree).unwrap();
                fs::remove_dir_all(output_tree).unwrap();
                assert!(client.take_warnings().is_empty());
                fs::write(fixture.root.join("reject-metadata"), b"true").unwrap();
                client
                    .upload(&source, "/metadata-warning", &cancel)
                    .unwrap();
                assert_eq!(
                    client.take_warnings().len(),
                    1,
                    "unsupported timestamp update must produce a nonfatal warning"
                );
                assert!(
                    client.take_warnings().is_empty(),
                    "warnings must be drained exactly once"
                );
                fs::remove_file(fixture.root.join("reject-metadata")).unwrap();
                client.delete("/metadata-warning", false).unwrap();
                assert!(
                    !client
                        .list("/")
                        .unwrap()
                        .iter()
                        .any(|e| e.name.starts_with(".ndr-transfer-"))
                );
                client.close().unwrap();
            }
            if protocol == "sftp" {
                let mut client = Client::connect(&config).unwrap();
                let destination = fixture.root.join("atomic-destination");
                fs::write(&destination, b"original local content").unwrap();
                let bytes = vec![0x5a; 1024 * 1024];
                fs::write(fixture.root.join("fail-read.bin"), &bytes).unwrap();
                fs::write(fixture.root.join("slow-read.bin"), &bytes).unwrap();
                let cancel = AtomicBool::new(false);
                assert!(
                    client
                        .download("/fail-read.bin", &destination, &cancel)
                        .is_err()
                );
                assert_eq!(fs::read(&destination).unwrap(), b"original local content");
                let signal = fixture.root.join("read-started");
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while !signal.exists() && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        cancel.store(true, Ordering::Relaxed);
                    });
                    assert!(
                        client
                            .download("/slow-read.bin", &destination, &cancel)
                            .is_err()
                    );
                });
                assert!(
                    signal.exists(),
                    "cancellation must occur after the remote read begins"
                );
                assert_eq!(fs::read(&destination).unwrap(), b"original local content");
                cancel.store(false, Ordering::Relaxed);
                fs::write(
                    fixture.root.join("failure-target"),
                    b"original remote content",
                )
                .unwrap();
                fs::write(fixture.root.join("inject-write-failure"), b"true").unwrap();
                let source = fixture.root.join("big-source");
                fs::write(&source, bytes).unwrap();
                assert!(client.upload(&source, "/failure-target", &cancel).is_err());
                fs::remove_file(fixture.root.join("inject-write-failure")).unwrap();
                assert_eq!(
                    fs::read(fixture.root.join("failure-target")).unwrap(),
                    b"original remote content"
                );
                assert!(
                    !client
                        .list("/")
                        .unwrap()
                        .iter()
                        .any(|e| e.name.starts_with(".ndr-transfer-"))
                );
                client.close().unwrap();
            }
        }
        let fixture = Fixture::start("ftps");
        let config = fixture.config("ftps");
        assert!(
            Client::connect(&config).is_err(),
            "self-signed certificate must not be trusted"
        );
        let mut plain = fixture.config("ftp");
        plain.password = "owned-test-only".into();
        assert!(
            Client::connect(&plain).is_err(),
            "TLS-required fixture must reject plaintext login"
        );
        for protocol in ["ftps", "ftps-euc-kr"] {
            let fixture = Fixture::start(protocol);
            let mut config = fixture.config(protocol);
            let cert = native_tls::Certificate::from_pem(
                &fs::read(fixture.root.join("fixture-cert.pem")).unwrap(),
            )
            .unwrap();
            let mut builder = native_tls::TlsConnector::builder();
            builder.add_root_certificate(cert);
            let connector = builder.build().unwrap();
            let encoding = if protocol.ends_with("-euc-kr") {
                encoding_rs::EUC_KR
            } else {
                encoding_rs::UTF_8
            };
            assert!(
                Ftp::connect(
                    &config,
                    socket(&config.host, config.port).unwrap(),
                    encoding,
                    Some(connector.clone())
                )
                .is_err(),
                "trusted certificate with wrong hostname must be rejected"
            );
            config.host = "localhost".into();
            for passive in [true, false] {
                eprintln!("TLS fixture {protocol}, passive={passive}");
                config.passive = passive;
                let ftp = Ftp::connect(
                    &config,
                    socket(&config.host, config.port).unwrap(),
                    encoding,
                    Some(connector.clone()),
                )
                .unwrap();
                let mut client = Client {
                    backend: Some(Backend::Ftp(Box::new(ftp))),
                    warnings: Vec::new(),
                };
                let source = fixture.root.join("tls-source");
                let output = fixture.root.join("tls-output");
                fs::write(&source, b"verified encrypted transfer").unwrap();
                let cancel = AtomicBool::new(false);
                client.upload(&source, "/암호화.txt", &cancel).unwrap();
                assert!(
                    client
                        .list("/")
                        .unwrap()
                        .iter()
                        .any(|e| e.name == "암호화.txt")
                );
                client.download("/암호화.txt", &output, &cancel).unwrap();
                assert_eq!(fs::read(output).unwrap(), b"verified encrypted transfer");
                client.delete("/암호화.txt", false).unwrap();
                client.close().unwrap();
            }
        }
    }

    #[test]
    #[ignore = "owned localhost integration; requires Python, pyftpdlib, paramiko, cryptography and pyOpenSSL"]
    fn owned_navigation_fixture() {
        for protocol in ["ftp", "sftp"] {
            let fixture = Fixture::start(protocol);
            let mut config = fixture.config(protocol);
            if protocol == "sftp" {
                let error: serde_json::Value = serde_json::from_str(&Client::connect(&config).err().unwrap()).unwrap();
                config.fingerprint = error["candidate"].as_str().unwrap().into();
            }
            let mut client = Client::connect(&config).unwrap();
            let home = client.pwd().unwrap();
            client.mkdir(&format!("{}/sub", home.trim_end_matches('/'))).unwrap();
            fs::write(fixture.root.join("note.txt"), b"x").unwrap();
            let entries = client.list(&home).unwrap();
            let note = entries.iter().find(|e| e.name == "note.txt").expect("listed");
            assert!(note.modified.is_some_and(|t| t > 1_600_000_000), "{protocol}: listing carries the modification time");
            let (inside, rejected) = client.change_directory(&home, "sub").unwrap();
            assert!(inside.ends_with("/sub") && rejected.is_none(), "{protocol}: {inside}");
            let (back, _) = client.change_directory(&inside, if protocol == "ftp" { ".." } else { &home }).unwrap();
            assert_eq!(back.trim_end_matches('/'), home.trim_end_matches('/'));
            if protocol == "ftp" {
                // C# NavigateAsync: even after a refused CWD, reads PWD and shows the reply text in the status line.
                let (still, rejected) = client.change_directory(&back, "missing").unwrap();
                assert_eq!(still, back);
                assert!(rejected.is_some());
            } else {
                assert!(client.change_directory(&back, "missing").is_err());
            }
            let _ = client.close();
        }
    }

    #[test]
    #[ignore = "owned localhost Ed25519 and encrypted ECDSA integration; requires Python paramiko and cryptography"]
    fn owned_sftp_key_fixture() {
        let fixture = Fixture::start("sftp-ed25519");
        let mut config = fixture.config("sftp-ed25519");
        config.password = "unused-password-must-not-authenticate".into();
        config.private_key = fixture.root.join("user-ed25519-key");
        let challenge = Client::connect(&config)
            .err()
            .expect("unknown Ed25519 host must require approval");
        let challenge: serde_json::Value = serde_json::from_str(&challenge)
            .unwrap_or_else(|_| panic!("Expected Ed25519 host challenge, got {challenge}"));
        assert_eq!(challenge["kind"], "unknown_host_key");
        let candidate = challenge["candidate"].as_str().unwrap();
        assert_eq!(
            candidate,
            fs::read_to_string(fixture.root.join("host-fingerprint.txt")).unwrap()
        );
        assert!(
            !fixture.root.join("auth.log").exists(),
            "host verification must precede public-key authentication"
        );
        config.fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode([0u8; 32]));
        let mismatch = Client::connect(&config).err().unwrap();
        assert!(mismatch.contains("host_key_mismatch"));
        assert!(!fixture.root.join("auth.log").exists());
        config.fingerprint = candidate.into();
        for (name, passphrase) in [
            ("user-ed25519-key", ""),
            ("user-ecdsa-encrypted-pem", "owned-key-passphrase"),
            ("user-ecdsa-encrypted-openssh", "owned-key-passphrase"),
        ] {
            config.private_key = fixture.root.join(name);
            config.key_passphrase = passphrase.into();
            let mut client = Client::connect(&config)
                .unwrap_or_else(|error| panic!("Key fixture {name} failed: {error}"));
            let source = fixture.root.join("key-source");
            let output = fixture.root.join("key-output");
            fs::write(&source, "키 인증 전송".as_bytes()).unwrap();
            let cancel = AtomicBool::new(false);
            client.upload(&source, "/키 인증.txt", &cancel).unwrap();
            client.download("/키 인증.txt", &output, &cancel).unwrap();
            assert_eq!(fs::read(&output).unwrap(), "키 인증 전송".as_bytes());
            client.delete("/키 인증.txt", false).unwrap();
            client.close().unwrap();
        }
        let passwords_before = fs::read_to_string(fixture.root.join("auth.log"))
            .unwrap()
            .lines()
            .filter(|line| *line == "password-auth")
            .count();
        config.key_passphrase = "incorrect-private-key-secret".into();
        let error = Client::connect(&config)
            .err()
            .expect("wrong key passphrase must fail");
        assert!(!error.contains("incorrect-private-key-secret"));
        assert!(!error.contains("unused-password-must-not-authenticate"));
        assert_eq!(
            fs::read_to_string(fixture.root.join("auth.log"))
                .unwrap()
                .lines()
                .filter(|line| *line == "password-auth")
                .count(),
            passwords_before,
            "an unreadable private key must not fall back to password authentication"
        );
    }

    /// Original FTP queue FUN_00467f0c (+0x40 resume, REST + RETR/STOR, SFTP seek) and FUN_00467dfc (overwrite if newer = time comparison).
    #[test]
    #[ignore = "owned localhost integration; requires Python, pyftpdlib, paramiko, cryptography and pyOpenSSL"]
    fn owned_resume_fixture() {
        use crate::fsops::ConflictChoice;
        let content: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for protocol in ["ftp", "ftp-euc-kr", "sftp"] {
            let fixture = Fixture::start(protocol);
            let mut config = fixture.config(protocol);
            if protocol == "sftp" {
                let error: serde_json::Value = serde_json::from_str(&Client::connect(&config).err().unwrap()).unwrap();
                config.fingerprint = error["candidate"].as_str().unwrap().into();
            }
            let mut client = Client::connect(&config).unwrap();
            let home = client.pwd().unwrap();
            let remote = |name: &str| format!("{}/{name}", home.trim_end_matches('/'));
            let cancel = AtomicBool::new(false);
            let local = fixture.root.join("local");
            fs::create_dir_all(&local).unwrap();
            assert!(client.can_resume(), "{protocol}: REST/seek must be available");
            // Resume download: only the first 30000 bytes exist locally.
            fs::write(fixture.root.join("big.bin"), &content).unwrap();
            fs::write(local.join("big.bin"), &content[..30_000]).unwrap();
            let mut asked = None;
            client
                .download_tree_with_conflicts(&remote("big.bin"), &local.join("big.bin"), &cancel, &mut |_, _, info| {
                    asked = Some((info.remote.size, info.can_resume, info.remote_is_source));
                    ConflictChoice::Resume
                }, &mut |_| {})
                .unwrap();
            assert_eq!(asked, Some((100_000, true, true)), "{protocol}");
            assert_eq!(fs::read(local.join("big.bin")).unwrap(), content, "{protocol}: resumed download");
            // Resume upload: only the first 40000 bytes exist remotely.
            fs::write(fixture.root.join("up.bin"), &content[..40_000]).unwrap();
            fs::write(local.join("up.bin"), &content).unwrap();
            client
                .upload_tree_with_conflicts(&local.join("up.bin"), &remote("up.bin"), &cancel, &mut |_, _, info| {
                    assert!(info.can_resume && !info.remote_is_source && info.remote.size == 40_000, "{protocol}");
                    ConflictChoice::Resume
                }, &mut |_| {})
                .unwrap();
            assert_eq!(fs::read(fixture.root.join("up.bin")).unwrap(), content, "{protocol}: resumed upload");
            // Overwrite if newer: with an older remote file, the download skips and the upload overwrites.
            fs::write(fixture.root.join("old.txt"), b"remote").unwrap();
            filetime::set_file_mtime(fixture.root.join("old.txt"), filetime::FileTime::from_unix_time(1_600_000_000, 0)).unwrap();
            fs::write(local.join("old.txt"), b"local").unwrap();
            client
                .download_tree_with_conflicts(&remote("old.txt"), &local.join("old.txt"), &cancel, &mut |_, _, _| ConflictChoice::OverwriteNewer, &mut |_| {})
                .unwrap();
            assert_eq!(fs::read(local.join("old.txt")).unwrap(), b"local", "{protocol}: an older remote file is skipped");
            client
                .upload_tree_with_conflicts(&local.join("old.txt"), &remote("old.txt"), &cancel, &mut |_, _, _| ConflictChoice::OverwriteNewer, &mut |_| {})
                .unwrap();
            assert_eq!(fs::read(fixture.root.join("old.txt")).unwrap(), b"local", "{protocol}: a newer local file overwrites");
            // Rename: download and upload under the name the asking side gave.
            client
                .download_tree_with_conflicts(&remote("old.txt"), &local.join("old.txt"), &cancel, &mut |_, _, info| {
                    *info.new_name.borrow_mut() = Some("renamed.txt".into());
                    ConflictChoice::Rename
                }, &mut |_| {})
                .unwrap();
            assert_eq!(fs::read(local.join("renamed.txt")).unwrap(), b"local", "{protocol}: download to the typed name");
            client
                .upload_tree_with_conflicts(&local.join("big.bin"), &remote("big.bin"), &cancel, &mut |_, _, info| {
                    *info.new_name.borrow_mut() = Some("copy.bin".into());
                    ConflictChoice::Rename
                }, &mut |_| {})
                .unwrap();
            assert_eq!(fs::read(fixture.root.join("copy.bin")).unwrap(), content, "{protocol}: upload to the typed name");
            let _ = client.close();
        }
    }
}

