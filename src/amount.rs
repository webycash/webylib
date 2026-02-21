//! Amount type with 8-decimal precision for Webcash
//!
//! Webcash amounts are stored as integers with 8 decimal places of precision.
//! For example, 1.00000000 webcash is stored as 100000000.
//!
//! The smallest unit is called a "wat" (equivalent to Bitcoin's satoshi).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// UTF-8 byte length of the ₩ symbol
const WEBCASH_SYMBOL_BYTES: usize = 3;

/// Amount type representing webcash values with 8 decimal places
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Amount {
    /// Amount in wats (smallest unit, 1e-8 webcash)
    pub wats: i64,
}

impl Amount {
    /// Number of decimal places for webcash amounts
    pub const DECIMALS: u32 = 8;

    /// The smallest unit (1e-8 webcash)
    pub const UNIT: i64 = 10_i64.pow(Self::DECIMALS);

    /// Zero amount
    pub const ZERO: Amount = Amount { wats: 0 };

    /// Create a new Amount from wats (smallest unit)
    pub const fn from_wats(wats: i64) -> Self {
        Amount { wats }
    }

    /// Create a new Amount from wats (smallest unit) - deprecated, use from_wats
    pub const fn from_sats(wats: i64) -> Self {
        Amount { wats }
    }

    /// Parse scientific notation (e.g., "1E-8" -> "0.00000001")
    /// This is a helper method for the FromStr trait implementation
    fn parse_scientific_notation(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(&['E', 'e'][..]).collect();
        if parts.len() != 2 {
            return Err(Error::amount("invalid scientific notation format"));
        }
        
        let coefficient: f64 = parts[0].parse()
            .map_err(|_| Error::amount("invalid coefficient in scientific notation"))?;
        let exponent: i32 = parts[1].parse()
            .map_err(|_| Error::amount("invalid exponent in scientific notation"))?;

        let result = if exponent >= 0 {
            // Positive exponent: multiply by 10^exponent
            coefficient * 10_f64.powi(exponent)
        } else {
            // Negative exponent: divide by 10^|exponent|
            coefficient / 10_f64.powi(-exponent)
        };
        
        Self::from_webcash(result)
    }

    /// Create Amount from webcash float value
    pub fn from_webcash(webcash: f64) -> Result<Self> {
        if webcash < 0.0 {
            return Err(Error::amount("negative amounts not allowed"));
        }

        // Convert to wats with proper rounding
        let wats = (webcash * Self::UNIT as f64).round() as i64;

        // Check for overflow
        if wats < 0 {
            return Err(Error::amount("amount too large"));
        }

        Ok(Amount { wats })
    }

    /// Convert to decimal string representation with default precision
    /// Use Display trait for standard formatting: `format!("{}", amount)`
    pub fn to_decimal_string(&self) -> String {
        self.to_string_with_decimals(Self::DECIMALS)
    }

    /// Convert to decimal string with specified decimal places
    pub fn to_string_with_decimals(&self, decimals: u32) -> String {
        if self.wats == 0 {
            return "0".to_string();
        }

        let divisor = 10_i64.pow(decimals);
        let integer_part = self.wats / divisor;
        let fractional_part = (self.wats % divisor).abs();

        if fractional_part == 0 {
            format!("{}", integer_part)
        } else {
            let fractional_str = format!("{:0width$}", fractional_part, width = decimals as usize);
            let trimmed = fractional_str.trim_end_matches('0');
            if trimmed.is_empty() {
                format!("{}", integer_part)
            } else {
                format!("{}.{}", integer_part, trimmed)
            }
        }
    }

    /// Get the amount in webcash units (divide by 10^8)
    pub fn to_webcash(&self) -> f64 {
        self.wats as f64 / Self::UNIT as f64
    }

    /// Convert to wats string representation (for webcash string format)
    /// Webcash strings use wats format: e10000:secret:... (not decimal format)
    pub fn to_wats_string(&self) -> String {
        self.wats.to_string()
    }

    /// Check if amount is valid (non-negative)
    pub fn is_valid(&self) -> bool {
        self.wats >= 0
    }

    /// Check if amount is zero
    pub fn is_zero(&self) -> bool {
        self.wats == 0
    }

    /// Check if amount is positive
    pub fn is_positive(&self) -> bool {
        self.wats > 0
    }

    /// Check if amount is negative
    pub fn is_negative(&self) -> bool {
        self.wats < 0
    }

    /// Get absolute value
    pub fn abs(&self) -> Self {
        Amount {
            wats: self.wats.abs(),
        }
    }

    /// Saturating addition
    pub fn saturating_add(&self, other: &Amount) -> Amount {
        Amount {
            wats: self.wats.saturating_add(other.wats),
        }
    }

    /// Saturating subtraction
    pub fn saturating_sub(&self, other: &Amount) -> Amount {
        Amount {
            wats: self.wats.saturating_sub(other.wats),
        }
    }

    /// Checked addition
    pub fn checked_add(&self, other: &Amount) -> Option<Amount> {
        self.wats.checked_add(other.wats).map(|wats| Amount { wats })
    }

    /// Checked subtraction
    pub fn checked_sub(&self, other: &Amount) -> Option<Amount> {
        self.wats.checked_sub(other.wats).map(|wats| Amount { wats })
    }

    /// Checked multiplication
    pub fn checked_mul(&self, other: i64) -> Option<Amount> {
        self.wats.checked_mul(other).map(|wats| Amount { wats })
    }

    /// Checked division
    pub fn checked_div(&self, other: i64) -> Option<Amount> {
        self.wats.checked_div(other).map(|wats| Amount { wats })
    }
}

impl Default for Amount {
    fn default() -> Self {
        Amount::ZERO
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_decimal_string())
    }
}

impl FromStr for Amount {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle empty strings
        if s.is_empty() {
            return Err(Error::amount("empty string"));
        }

        // Handle scientific notation first (before stripping prefixes)
        if s.contains('E') || s.contains('e') {
            return Self::parse_scientific_notation(s);
        }

        // Handle strings that start with 'e' (webcash format)
        let s = if let Some(stripped) = s.strip_prefix('e') {
            stripped
        } else if s.starts_with('₩') {
            &s[WEBCASH_SYMBOL_BYTES..]
        } else {
            s
        };

        // Handle special case of zero
        if s == "0" {
            return Ok(Amount::ZERO);
        }

        // Split into integer and fractional parts
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() > 2 {
            return Err(Error::amount("too many decimal points"));
        }

        let integer_part = parts[0];
        let fractional_part = if parts.len() == 2 { parts[1] } else { "" };

        // Validate integer part
        if integer_part.is_empty() && !fractional_part.is_empty() {
            return Err(Error::amount("missing integer part"));
        }

        // Parse integer part
        let mut wats = if integer_part.is_empty() {
            0
        } else {
            integer_part.parse::<i64>().map_err(|_| Error::amount("invalid integer part"))?
        };

        // Handle fractional part
        if !fractional_part.is_empty() {
            if fractional_part.len() > Amount::DECIMALS as usize {
                return Err(Error::amount("too many decimal places"));
            }

            // Parse fractional part
            let frac_value = fractional_part.parse::<i64>().map_err(|_| Error::amount("invalid fractional part"))?;

            // Calculate the multiplier for the fractional part
            let multiplier = 10_i64.pow(Amount::DECIMALS - fractional_part.len() as u32);
            let fractional_sats = frac_value * multiplier;

            // Add fractional part to wats
            wats = wats.checked_mul(Amount::UNIT).and_then(|s| s.checked_add(fractional_sats))
                .ok_or_else(|| Error::amount("amount too large"))?;
        } else {
            // No fractional part, multiply by UNIT
            wats = wats.checked_mul(Amount::UNIT)
                .ok_or_else(|| Error::amount("amount too large"))?;
        }

        // Check for negative amounts (not allowed in webcash)
        if wats < 0 {
            return Err(Error::amount("negative amounts not allowed"));
        }

        Ok(Amount { wats })
    }
}

impl std::ops::Add for Amount {
    type Output = Amount;

    fn add(self, other: Amount) -> Amount {
        Amount {
            wats: self.wats.saturating_add(other.wats),
        }
    }
}

impl std::ops::Sub for Amount {
    type Output = Amount;

    fn sub(self, other: Amount) -> Amount {
        Amount {
            wats: self.wats.saturating_sub(other.wats),
        }
    }
}

impl std::ops::Mul<i64> for Amount {
    type Output = Amount;

    fn mul(self, rhs: i64) -> Amount {
        Amount {
            wats: self.wats.saturating_mul(rhs),
        }
    }
}

impl std::ops::Div<i64> for Amount {
    type Output = Amount;

    /// Division operator for Amount
    /// 
    /// # Panics
    /// Panics if divisor is zero, following Rust's standard integer division behavior
    fn div(self, rhs: i64) -> Amount {
        Amount {
            wats: self.wats / rhs, // Let Rust handle division by zero with standard behavior
        }
    }
}

impl std::ops::AddAssign for Amount {
    fn add_assign(&mut self, other: Amount) {
        self.wats = self.wats.saturating_add(other.wats);
    }
}

impl std::ops::SubAssign for Amount {
    fn sub_assign(&mut self, other: Amount) {
        self.wats = self.wats.saturating_sub(other.wats);
    }
}

