use crate::{routes::info::InfoResult, tests::setup::new_test_server, Args};
use katana_runner::RunnerCtx;

#[tokio::test(flavor = "multi_thread")]
#[katana_runner::test(accounts = 10)]
async fn test_info(_sequencer: &RunnerCtx) {
    let args = Args::default().with_secret_key(420);
    let server = new_test_server(&args).await;

    let info = server.get("/info").await;
    let result = info.json::<InfoResult>();

    assert!(
        result.public_key_x == "0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4",
        "invalid public_key_x"
    );
    assert!(
        result.public_key_y == "0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232",
        "invalid public_key_y"
    );
}
