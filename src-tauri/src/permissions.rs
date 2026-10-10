//! What each permission grants. The permissions are the app's: a plugin only picks from this list (and says what it is of,
//! when the permission asks: the hosts, the file, the command), and the words that say what the permission grants are the
//! app's too, the same for every plugin that asks for it. A plugin borrows what the app can do; it does not get to describe it.

/// (name, what it grants, what it grants when it is of something: `{}` is that)
const GRANTS: &[(&str, &str, &str)] = &[
    ("windows", "See which programs have a window open, which one is in front, and what the windows are titled.", ""),
    ("processes", "See which programs have a window open, and which one is in front.", ""),
    ("titles", "Read the titles of the windows (a browser's title is its page).", ""),
    ("performance", "Read how much CPU, memory and GPU each program uses, from Windows.", ""),
    ("idle time", "Know how long it has been since the keyboard or mouse was last used (never what was typed).", ""),
    (
        "read files and folders",
        "Read files and folders on this PC.",
        "Read files and folders on this PC: {}.",
    ),
    (
        "network",
        "Reach the internet, and only the addresses the plugin names.",
        "Reach the internet, and only these addresses: {}.",
    ),
    (
        "use CLI",
        "Run command-line programs on this PC.",
        "Run a command-line program on this PC: {}.",
    ),
    ("notifications", "Read the notifications that other apps show (their title and text).", ""),
    ("browser extension", "Receive what the NADI browser extension sends about your open tabs.", ""),
    ("media", "See what is playing: the title, the artist and how far along it is.", ""),
    ("media pictures", "See the cover picture of what is playing.", ""),
    ("media control", "Press play, pause, next and previous for the player.", ""),
    ("audio", "Hear what is playing as levels and bands (never the sound itself).", ""),
    ("system", "See how busy the CPU and the memory are.", ""),
    ("calendar", "Read the events of the island's calendar (whichever plugin brought them).", ""),
    ("clock", "Know the date and the time.", ""),
];

/// What the permission grants, for a plugin that asks for it (of `detail`, when it says), or `None` for a name the app
/// does not know.
pub fn grants(name: &str, detail: &str) -> Option<String> {
    let (_, plain, of) = GRANTS.iter().find(|(n, _, _)| *n == name)?;
    Some(if detail.is_empty() || of.is_empty() { plain.to_string() } else { of.replace("{}", detail) })
}

#[cfg(test)]
pub fn known(name: &str) -> bool {
    GRANTS.iter().any(|(n, _, _)| *n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_says_what_it_grants_and_of_what() {
        assert_eq!(grants("use CLI", "").unwrap(), "Run command-line programs on this PC.");
        assert_eq!(grants("use CLI", "git").unwrap(), "Run a command-line program on this PC: git.");
        // (a permission that is not of anything says the same whatever it is told)
        assert_eq!(grants("windows", "x"), grants("windows", ""));
        assert!(grants("root", "").is_none());
    }

    #[test]
    fn every_permission_has_words_and_a_unique_name() {
        let mut names: Vec<&str> = GRANTS.iter().map(|g| g.0).collect();
        assert!(GRANTS.iter().all(|(n, plain, of)| !n.is_empty() && plain.ends_with('.') && (of.is_empty() || of.contains("{}"))));
        names.sort();
        names.dedup();
        assert_eq!(names.len(), GRANTS.len());
    }
}
