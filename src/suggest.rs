//! "Unknown X" errors that name the likely one: `unknown seat 'wrk' (maybe you meant 'work'; known: work, personal)`.
//! One place for the edit distance and the wording, so every lookup of a seat, preset or group by name says the
//! same thing.

/// Edit distance between two names, ignoring ASCII case.
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(!ca.eq_ignore_ascii_case(cb));
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The names close enough to `name` to be a typo of it, closest first. Short names tolerate one slip, longer two,
/// and never the whole name (every one-letter name is one edit from every other).
pub fn closest<'a>(name: &str, names: &[&'a str]) -> Vec<&'a str> {
    let limit = if name.chars().count() <= 3 { 1 } else { 2 };
    let mut hits: Vec<(usize, &str)> = names
        .iter()
        .map(|n| (distance(name, n), *n))
        .filter(|(d, n)| *d <= limit && *d < name.chars().count().max(n.chars().count()))
        .collect();
    hits.sort_by_key(|(d, n)| (*d, n.len()));
    hits.dedup_by_key(|(_, n)| *n);
    hits.into_iter().map(|(_, n)| n).take(3).collect()
}

/// `unknown <kind> '<name>'`, with what it may have meant and what exists.
pub fn unknown(kind: &str, name: &str, names: &[&str]) -> String {
    let mut msg = format!("unknown {kind} '{name}'");
    let near = closest(name, names);
    let mut extra = Vec::new();
    if !near.is_empty() {
        let quoted: Vec<String> = near.iter().map(|n| format!("'{n}'")).collect();
        extra.push(format!("maybe you meant {}", quoted.join(" or ")));
    }
    if !names.is_empty() {
        extra.push(format!("known: {}", names.join(", ")));
    }
    if !extra.is_empty() {
        msg += &format!(" ({})", extra.join("; "));
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_the_close_name_and_lists_the_rest() {
        let names = ["work", "frontier", "personal"];
        assert_eq!(
            unknown("seat", "wrk", &names),
            "unknown seat 'wrk' (maybe you meant 'work'; known: work, frontier, personal)"
        );
        assert_eq!(
            unknown("seat", "persnal", &names),
            "unknown seat 'persnal' (maybe you meant 'personal'; known: work, frontier, personal)"
        );
        assert_eq!(
            unknown("seat", "banana", &names),
            "unknown seat 'banana' (known: work, frontier, personal)"
        );
        assert_eq!(unknown("group", "x", &[]), "unknown group 'x'");
    }

    #[test]
    fn one_letter_names_do_not_match_each_other() {
        assert!(closest("c", &["a", "b"]).is_empty());
        assert_eq!(closest("WORK", &["work"]), vec!["work"]);
    }
}
