use inventory::message::check;

#[test]
fn zero_is_refused() {
    assert!(check(0).is_err());
}

#[test]
fn a_positive_quantity_passes() {
    assert_eq!(check(3), Ok(3));
}
