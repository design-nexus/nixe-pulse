//! Native statistics with bounded line plots for Telegram graph payloads.
use crate::{
    features::{self, alive},
    groups::note,
};
use gtk::prelude::*;
use serde_json::{Value, json};
pub fn open(chat: i64, message: Option<i64>) {
    features::load("Statistics", move |s, d, b| async move {
        let data = if let Some(id) = message {
            if s.call(
                "getMessageProperties",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?["can_get_statistics"]
                != true
            {
                return Err("Telegram does not permit statistics for this message.".into());
            }
            s.call(
                "getMessageStatistics",
                json!({
                "chat_id":chat,"message_id":id,"is_dark":true}
                ),
            )
            .await?
        } else {
            let c = crate::groups::context(&s, chat).await?;
            let full = s
                .call(
                    "getSupergroupFullInfo",
                    json!({
                    "supergroup_id":c.chat["type"]["supergroup_id"]}
                    ),
                )
                .await?;
            if full["can_get_statistics"] != true {
                return Err("Statistics are unavailable for this account or chat.".into());
            }
            s.call(
                "getChatStatistics",
                json!({
                "chat_id":chat,"is_dark":true}
                ),
            )
            .await?
        };
        if !alive(&s, &d) {
            return Ok(());
        }
        if let Some(fields) = data.as_object() {
            for (name, value) in fields {
                if name.starts_with('@') {
                    continue;
                }
                note(&b, &features::pretty(&name.replace('_', " ")));
                let mut value = value.clone();
                if value["@type"] == "statisticalGraphAsync" {
                    value = s
                        .call(
                            "getStatisticalGraph",
                            json!({
                            "chat_id":chat,"token":value["token"],"x":0}
                            ),
                        )
                        .await?;
                }
                if !alive(&s, &d) {
                    return Ok(());
                }
                if value["@type"] == "statisticalGraphData" {
                    let chart = value["json_data"]
                        .as_str()
                        .and_then(|v| serde_json::from_str::<Value>(v).ok())
                        .unwrap_or(Value::Null);
                    graph(&b, &chart);
                } else {
                    note(&b, &features::summary(&value));
                }
            }
        }
        Ok(())
    });
}
fn series(chart: &Value) -> Vec<(String, Vec<f64>)> {
    chart["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|column| {
            let a = column.as_array()?;
            let key = a.first()?.as_str()?;
            if chart["types"][key] == "x" {
                return None;
            }
            let values = a
                .iter()
                .skip(1)
                .filter_map(Value::as_f64)
                .filter(|n| n.is_finite())
                .take(10000)
                .collect::<Vec<_>>();
            if values.is_empty() {
                return None;
            }
            Some((chart["names"][key].as_str().unwrap_or(key).to_string(), values))
        })
        .take(12)
        .collect()
}
fn graph(body: &gtk::Box, chart: &Value) {
    let data = series(chart);
    if data.is_empty() {
        note(body, "No graph data available.");
        return;
    }
    note(body, &data.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" · "));
    let area = gtk::DrawingArea::new();
    area.set_content_width(360);
    area.set_content_height(180);
    area.set_hexpand(true);
    area.set_draw_func(move |_, cr, width, height| {
        let max = data.iter().flat_map(|(_, v)| v).copied().fold(0., f64::max).max(1.);
        let colors = [(0.32, 0.65, 0.95), (0.3, 0.8, 0.5), (0.95, 0.65, 0.25), (0.85, 0.35, 0.6)];
        for (i, (_, v)) in data.iter().enumerate() {
            let (r, g, b) = colors[i % colors.len()];
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(1.5);
            for (j, value) in v.iter().enumerate() {
                let x = 8. + j as f64 / (v.len().saturating_sub(1).max(1)) as f64 * (width as f64 - 16.);
                let y = height as f64 - 8. - value / max * (height as f64 - 16.);
                if j == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
            }
            let _ = cr.stroke();
        }
    });
    body.append(&area);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_chart_series() {
        let v = series(&json!({
        "columns":[["x",1,2],["a",5,7]],"types":{
        "x":"x","a":"line"}
        ,"names":{
        "a":"Views"}
        }
        ));
        assert_eq!(v[0].0, "Views");
        assert_eq!(v[0].1, vec![5., 7.]);
    }
}
