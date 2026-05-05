//! Compile-time assertion that wrapper futures on `NifiClient` are `Send`.
//!
//! Without this test, a future PR could accidentally introduce a `!Send`
//! capture (e.g. a `Rc<...>` in a wrapper helper) and silently re-create the
//! LocalSet workaround. We pin the constraint here so that regression
//! breaks the build, not runtime behaviour.

#![allow(dead_code)]

fn assert_send<T: Send>(_: &T) {}

fn _check(client: &nifi_lens::client::NifiClient) {
    assert_send(&client.system_diagnostics(true));
    assert_send(&client.controller_status());
    assert_send(&client.bulletin_board(None, Some(100)));
    assert_send(&client.about());
    assert_send(&client.root_pg_status());
    assert_send(&client.controller_services_snapshot());
    assert_send(&client.reporting_tasks_snapshot());
    assert_send(&client.cluster_nodes());
}

#[test]
fn send_regression_compiles() {
    // The real assertion is the compile of `_check` above. This test
    // exists so `cargo test` enumerates the file.
}
