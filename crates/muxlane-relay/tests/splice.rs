use futures_util::{SinkExt, StreamExt};
use muxlane_relay::Relay;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn host_dials_a_channel_per_phone_and_exchanges_frames() {
    let bind = spawn_relay().await;

    let (mut host, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/host/machine_a"))
        .await
        .unwrap();
    host.send(Message::Text("12345678".into())).await.unwrap();
    let ack = host.next().await.unwrap().unwrap();
    assert_eq!(ack, Message::Text("ok".into()));

    let (mut phone, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/pair/12345678"))
        .await
        .unwrap();
    let dial = host.next().await.unwrap().unwrap();
    let chan = dial
        .to_text()
        .unwrap()
        .strip_prefix(muxlane_relay::DIAL_FRAME_PREFIX)
        .expect("dial frame")
        .to_string();

    // host 按控制帧指示开数据通道
    let (mut data, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/chan/{chan}"))
        .await
        .unwrap();

    phone
        .send(Message::Text("{\"id\":1,\"method\":\"state.list\"}".into()))
        .await
        .unwrap();
    let forwarded = data.next().await.unwrap().unwrap();
    assert_eq!(
        forwarded,
        Message::Text("{\"id\":1,\"method\":\"state.list\"}".into())
    );

    data.send(Message::Text("{\"id\":1,\"result\":{}}".into()))
        .await
        .unwrap();
    let reply = phone.next().await.unwrap().unwrap();
    assert_eq!(reply, Message::Text("{\"id\":1,\"result\":{}}".into()));

    // 同一 host 可同时挂第二条通道（桌面客户端多连接）
    let (mut phone2, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/phone/machine_a"))
        .await
        .unwrap();
    let dial2 = host.next().await.unwrap().unwrap();
    let chan2 = dial2
        .to_text()
        .unwrap()
        .strip_prefix(muxlane_relay::DIAL_FRAME_PREFIX)
        .expect("dial frame")
        .to_string();
    let (mut data2, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/chan/{chan2}"))
        .await
        .unwrap();
    phone2.send(Message::Text("second".into())).await.unwrap();
    let forwarded2 = data2.next().await.unwrap().unwrap();
    assert_eq!(forwarded2, Message::Text("second".into()));
}

#[tokio::test]
async fn expired_or_unknown_pair_codes_are_rejected() {
    let bind = spawn_relay().await;
    let (mut phone, _) = tokio_tungstenite::connect_async(format!("ws://{bind}/pair/00000000"))
        .await
        .unwrap();
    let closed = phone.next().await;
    assert!(matches!(
        closed,
        None | Some(Ok(Message::Close(_))) | Some(Err(_))
    ));
}

async fn spawn_relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let bind = addr.to_string();
    tokio::spawn({
        let bind = bind.clone();
        async move {
            Relay::new().serve(&bind).await.unwrap();
        }
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&bind).await.is_ok() {
            return bind;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("relay never became ready");
}
