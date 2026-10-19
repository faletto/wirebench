//! Message catalog. Every string the shell shows a user lives here so it can be reviewed
//! for provenance (our own wording, not copied vendor text) and later localised.

pub const INVALID: &str = "% Unrecognized input; error at the '^' marker.";
pub const INCOMPLETE: &str = "% Command is incomplete.";
pub const CR: &str = "<cr>";

pub fn ambiguous(token: &str) -> String {
    format!("% Ambiguous abbreviation: \"{token}\"")
}
