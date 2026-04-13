use webylib::Error;

#[test]
fn test_error_creation() {
    let parse_err = Error::parse("invalid format");
    assert!(matches!(parse_err, Error::Parse { .. }));

    let amount_err = Error::amount("too many decimals");
    assert!(matches!(amount_err, Error::Amount { .. }));

    let wallet_err = Error::wallet("insufficient funds");
    assert!(matches!(wallet_err, Error::Wallet { .. }));
}

#[test]
fn test_error_display() {
    let err = Error::parse("test message");
    let display = format!("{}", err);
    assert_eq!(display, "Parse error: test message");

    let err = Error::insufficient_funds("100", "50");
    let display = format!("{}", err);
    assert_eq!(display, "Insufficient funds: needed 100, available 50");
}
