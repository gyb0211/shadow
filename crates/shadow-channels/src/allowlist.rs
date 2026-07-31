#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// Exact `==` match.
    Sensitive,
    /// `eq_ignore_ascii_case` — IRC nicks, Matrix MXIDs.
    CaseInsensitive,
}
pub fn is_user_allowed(allowed: &[String], user: &str, mode: Match) -> bool {
    if allowed.iter().any(|u| u == "*") {
        return true;
    }
    match mode {
        Match::Sensitive => allowed.iter().any(|u| u == user),
        Match::CaseInsensitive => allowed.iter().any(|u| u.eq_ignore_ascii_case(user)),
    }
}
