//! Default encrypted-folder names from the Paleobiology Database's accepted
//! non-avian dinosaur genera (via DinoPages, CC BY 4.0), plus familiar Korean
//! transliterations. This is UI variety, not security-grade randomness.
use std::{path::Path, sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};

const GENERA: &str = include_str!("../../Resources/dinosaur_genera.txt");
const KOREAN: &str = include_str!("../../Resources/dinosaur_names_ko.tsv");
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn next_random() -> u64 {
    let ticks = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    let mut value = ticks ^ (u64::from(std::process::id()) << 33)
        ^ SEQUENCE.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E3779B97F4A7C15);
    // SplitMix64 is sufficient for varied suggestions, not for keys/passwords.
    value = value.wrapping_add(0x9E3779B97F4A7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}

fn candidates() -> (&'static [&'static str], &'static [&'static str]) {
    use std::sync::OnceLock;
    static LISTS: OnceLock<(Vec<&'static str>, Vec<&'static str>)> = OnceLock::new();
    let lists = LISTS.get_or_init(|| {
        let genera = GENERA.lines().map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>();
        let korean = KOREAN.lines().filter_map(|line| line.split_once('\t').map(|(name, _)| name))
            .filter(|name| !name.is_empty()).collect::<Vec<_>>();
        (genera, korean)
    });
    (&lists.0, &lists.1)
}

fn available(parent: &Path, name: &str) -> bool {
    !parent.join(format!("{name}.ndr")).exists()
}

pub(super) fn suggest(parent: &Path) -> String {
    let (genera, korean) = candidates();
    // Usually show a Korean dinosaur name, but make all genus names reachable.
    for attempt in 0..128 {
        let random = next_random();
        let pool = if attempt % 4 != 3 && !korean.is_empty() { korean } else { genera };
        let candidate = pool[random as usize % pool.len()];
        if available(parent, candidate) { return candidate.to_owned(); }
    }
    // Exhaustion is unlikely, but never silently replace a user's vault.
    let name = korean.first().copied().unwrap_or("Dinosaur");
    for number in 2..u32::MAX {
        let candidate = format!("{name} ({number})");
        if available(parent, &candidate) { return candidate; }
    }
    format!("Dinosaur-{}", next_random())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn full_dinosaur_pool_is_unique_and_korean_transliterations_are_grounded() {
        let (genera, korean) = candidates();
        assert_eq!(genera.len(), 1605);
        assert_eq!(korean.len(), 179);
        let genus_set: HashSet<_> = genera.iter().copied().collect();
        assert_eq!(genus_set.len(), genera.len());
        let korean_set: HashSet<_> = korean.iter().copied().collect();
        assert_eq!(korean_set.len(), korean.len());
        for row in KOREAN.lines() {
            let (name, genus) = row.split_once('\t').unwrap();
            assert!(genus_set.contains(genus), "Unknown genus: {name} -> {genus}");
            assert!(name.chars().all(|ch| ('가'..='힣').contains(&ch)));
        }
        assert!(korean.contains(&"티라노사우루스"));
        assert!(genera.contains(&"Tyrannosaurus"));
    }

    #[test]
    fn random_suggestion_is_safe_unique_and_skips_existing_vaults() {
        let root = std::env::temp_dir().join(format!("ndr-dinosaur-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut seen = HashSet::new();
        for _ in 0..40 {
            let name = suggest(&root);
            assert!(!name.is_empty());
            assert!(!name.contains(&['/', '\\', ':'][..]));
            assert!(!root.join(format!("{name}.ndr")).exists());
            seen.insert(name);
        }
        assert!(seen.len() >= 4, "Suggestions must vary");
        let selected = suggest(&root);
        std::fs::write(root.join(format!("{selected}.ndr")), b"existing").unwrap();
        for _ in 0..10 {
            assert_ne!(suggest(&root), selected, "An existing vault must not be offered as new");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
