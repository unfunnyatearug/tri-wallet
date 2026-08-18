use anyhow::{anyhow, Result};

pub fn warn(msg: &str) {
    eprintln!("WARNING: {msg}");
}

pub fn note(msg: &str) {
    eprintln!("NOTE: {msg}");
}

pub fn confirm(prompt: &str) -> Result<bool> {
    use std::io::{self, Write};
    print!("{prompt} [y/N]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

pub fn prompt_line(prompt: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Parse a decimal amount into base units. decimals = 8 for BTC, 9 for SOL, 6 for USDC.
pub fn parse_amount(s: &str, decimals: u32) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty amount"));
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((a, b)) => (a, b),
        None => (s, ""),
    };
    if !int_part.chars().all(|c| c.is_ascii_digit()) || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(anyhow!("amount must be a plain decimal number, got '{s}'"));
    }
    if frac_part.len() > decimals as usize {
        return Err(anyhow!(
            "too many decimal places: this asset supports {decimals}"
        ));
    }
    let int_v: u128 = if int_part.is_empty() {
        0
    } else {
        int_part.parse()?
    };
    let mut frac_v: u128 = if frac_part.is_empty() {
        0
    } else {
        frac_part.parse()?
    };
    for _ in 0..(decimals as usize - frac_part.len()) {
        frac_v *= 10;
    }
    let total = int_v
        .checked_mul(10u128.pow(decimals))
        .and_then(|v| v.checked_add(frac_v))
        .ok_or_else(|| anyhow!("amount too large"))?;
    u64::try_from(total).map_err(|_| anyhow!("amount too large"))
}

pub fn format_amount(v: u64, decimals: u32) -> String {
    let d = 10u64.pow(decimals);
    let int = v / d;
    let frac = v % d;
    if frac == 0 {
        return int.to_string();
    }
    let mut frac_s = format!("{:0width$}", frac, width = decimals as usize);
    while frac_s.ends_with('0') {
        frac_s.pop();
    }
    format!("{int}.{frac_s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_round_trip() {
        assert_eq!(parse_amount("0.0001", 8).unwrap(), 10_000);
        assert_eq!(parse_amount("1", 8).unwrap(), 100_000_000);
        assert_eq!(parse_amount("12.5", 6).unwrap(), 12_500_000);
        assert_eq!(format_amount(10_000, 8), "0.0001");
        assert_eq!(format_amount(100_000_000, 8), "1");
        assert_eq!(format_amount(12_500_000, 6), "12.5");
    }

    #[test]
    fn amounts_reject_bad_input() {
        assert!(parse_amount("0.123456789", 8).is_err());
        assert!(parse_amount("1,5", 8).is_err());
        assert!(parse_amount("abc", 8).is_err());
    }
}
