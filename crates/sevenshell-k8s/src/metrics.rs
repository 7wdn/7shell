//! Reading the numbers `metrics.k8s.io` reports.
//!
//! A metrics quantity is a decimal number with a unit glued on: `12m` cores, `20Mi` bytes. The
//! arithmetic is trivial and the spelling is not, so the spelling is what is tested here — a table of
//! the forms that actually appear in a cluster's metrics, because a column that says `0` for a pod
//! using 12 millicores is worse than no column at all.
//!
//! Nothing in this module touches the network or a wire type: the interface reads the API's JSON
//! straight and these are the two conversions a column needs.
#![allow(dead_code)]

/// The decimal suffixes a quantity may carry, longest first so `Mi` is not read as `M` followed by
/// an `i` nobody expected.
const SUFFIXES: [(&str, f64); 15] = [
    ("Ki", 1024.0),
    ("Mi", 1024.0 * 1024.0),
    ("Gi", 1024.0 * 1024.0 * 1024.0),
    ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
    ("Pi", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
    ("Ei", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
    ("n", 1e-9),
    ("u", 1e-6),
    ("m", 1e-3),
    ("k", 1e3),
    ("M", 1e6),
    ("G", 1e9),
    ("T", 1e12),
    ("P", 1e15),
    ("E", 1e18),
];

/// A CPU usage in millicores, the unit every Kubernetes console shows.
///
/// Rounded rather than truncated: `1500n` is 0.0015 of a core, and a column that shows it as `0`
/// while the pod is doing work is a column nobody trusts. Values this small still round to zero,
/// which is the honest answer for "less than a thousandth of a core".
pub fn parse_cpu_millicores(quantity: &str) -> Option<u64> {
    let cores = parse(quantity)?;
    if cores < 0.0 {
        return None;
    }
    Some((cores * 1000.0).round() as u64)
}

/// A memory usage in bytes.
pub fn parse_memory_bytes(quantity: &str) -> Option<u64> {
    let bytes = parse(quantity)?;
    if bytes < 0.0 {
        return None;
    }
    Some(bytes.round() as u64)
}

/// A CPU usage in the form a Kubernetes console writes it: whole cores, or millicores.
pub fn format_cpu(millicores: u64) -> String {
    if millicores.is_multiple_of(1000) {
        format!("{}", millicores / 1000)
    } else {
        format!("{millicores}m")
    }
}

/// A memory usage in binary units, which is how a cluster's own numbers are spelled.
///
/// Falls back to bytes rather than printing a fraction: a column that says `1.5Gi` next to a server
/// that says `1536Mi` invites somebody to go looking for the difference.
pub fn format_memory(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("Ti", 1024 * 1024 * 1024 * 1024),
        ("Gi", 1024 * 1024 * 1024),
        ("Mi", 1024 * 1024),
        ("Ki", 1024),
    ];
    for (suffix, unit) in UNITS {
        if bytes >= unit && bytes.is_multiple_of(unit) {
            return format!("{}{suffix}", bytes / unit);
        }
    }
    format!("{bytes}B")
}

/// The number a quantity spells, in its base unit.
fn parse(quantity: &str) -> Option<f64> {
    let text = quantity.trim();
    if text.is_empty() {
        return None;
    }
    // `1e3` and `0.5` are quantities with no suffix at all, and a plain parse already reads them.
    if let Ok(value) = text.parse::<f64>() {
        return value.is_finite().then_some(value);
    }
    for (suffix, factor) in SUFFIXES {
        if let Some(number) = text.strip_suffix(suffix) {
            let value: f64 = number.trim().parse().ok()?;
            return value.is_finite().then_some(value * factor);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_quantities_become_millicores() {
        // Every form a live cluster's metrics has been seen to use, including the ones that look
        // like a typo and are not.
        let table = [
            ("0", Some(0)),
            ("1", Some(1000)),
            ("0.5", Some(500)),
            ("250m", Some(250)),
            ("12m", Some(12)),
            ("1500n", Some(0)),
            ("100000n", Some(0)),
            ("1000000n", Some(1)),
            ("250u", Some(0)),
            ("250000u", Some(250)),
            ("1e3", Some(1_000_000)),
            ("15.5m", Some(16)),
            ("", None),
            ("12mm", None),
            ("-1", None),
            ("lots", None),
        ];
        for (quantity, expected) in table {
            assert_eq!(
                parse_cpu_millicores(quantity),
                expected,
                "`{quantity}` should read as {expected:?} millicores"
            );
        }
    }

    #[test]
    fn memory_quantities_become_bytes() {
        let table = [
            ("0", Some(0)),
            ("128974848", Some(128_974_848)),
            ("129M", Some(129_000_000)),
            ("123Mi", Some(123 * 1024 * 1024)),
            ("1.5Gi", Some(1024 * 1024 * 1024 + 512 * 1024 * 1024)),
            ("1Ki", Some(1024)),
            ("1Gi", Some(1024 * 1024 * 1024)),
            ("2T", Some(2_000_000_000_000)),
            ("1E", Some(1_000_000_000_000_000_000)),
            ("8e6", Some(8_000_000)),
            ("bogus", None),
            ("10MiB", None),
            ("", None),
        ];
        for (quantity, expected) in table {
            assert_eq!(
                parse_memory_bytes(quantity),
                expected,
                "`{quantity}` should read as {expected:?} bytes"
            );
        }
    }

    #[test]
    fn a_whole_core_is_written_without_a_suffix() {
        assert_eq!(format_cpu(1000), "1");
        assert_eq!(format_cpu(250), "250m");
        assert_eq!(format_cpu(0), "0");
    }

    #[test]
    fn memory_is_written_in_the_binary_unit_that_divides_it() {
        assert_eq!(format_memory(128 * 1024 * 1024), "128Mi");
        assert_eq!(format_memory(1024 * 1024 * 1024), "1Gi");
        assert_eq!(format_memory(1000), "1000B");
        assert_eq!(format_memory(0), "0B");
    }
}
