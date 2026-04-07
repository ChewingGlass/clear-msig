use serde::Serialize;

/// Print a JSON value to stdout.
pub fn print_json<T: Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}
