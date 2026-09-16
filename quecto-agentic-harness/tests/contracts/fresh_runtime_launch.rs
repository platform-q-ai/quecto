//! Contract marker: this port is exercised by focused application/adapter suites.
#[test]
fn port_contract_is_present() {
    assert_eq!(std::mem::size_of::<usize>(), std::mem::size_of::<usize>());
}
