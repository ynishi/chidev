use inventory::stock::{fits, spare};

#[test]
fn a_load_under_the_limit_fits() {
    assert!(fits(1, 5));
}

#[test]
fn a_load_over_the_limit_does_not_fit() {
    assert!(!fits(9, 5));
}

#[test]
fn spare_is_the_limit_less_the_used() {
    assert_eq!(spare(5, 2), 3);
}
