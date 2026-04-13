use webylib::ChainCode;

#[test]
fn test_chain_codes() {
    assert_eq!(ChainCode::Receive.as_u64(), 0);
    assert_eq!(ChainCode::Pay.as_u64(), 1);
    assert_eq!(ChainCode::Change.as_u64(), 2);
    assert_eq!(ChainCode::Mining.as_u64(), 3);

    assert_eq!(ChainCode::from_u64(0), Some(ChainCode::Receive));
    assert_eq!(ChainCode::from_u64(1), Some(ChainCode::Pay));
    assert_eq!(ChainCode::from_u64(2), Some(ChainCode::Change));
    assert_eq!(ChainCode::from_u64(3), Some(ChainCode::Mining));
    assert_eq!(ChainCode::from_u64(4), None);
}
