use tokio_tungstenite::connect_async;
use futures::{SinkExt, StreamExt};
use serde_json::Value;

pub async fn run_market_ws<F>(
    url: &str,
    token_ids: &[String],
    mut on_update: F,
) -> anyhow::Result<()>
where
    F: FnMut(Value) + Send,
{
    let (mut ws, _) = connect_async(url).await?;

    ws.send(serde_json::json!({
        "type": "market",
        "assets_ids": token_ids
    }).to_string().into()).await?;

    while let Some(msg) = ws.next().await {
        let msg = msg?;
        if msg.is_text() {
            let v: Value = serde_json::from_str(msg.to_text()?)?;
            on_update(v);
        }
    }
    Ok(())
}

