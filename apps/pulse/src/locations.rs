//! GeoClue fixes and explicit automatic updates for native outgoing live locations.
use crate::{
    actions::{self, Scope},
    features::{self, button},
    groups::note,
    window,
};
use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
thread_local! {
static ACTIVE:RefCell<Vec<Value>>=const{
RefCell::new(Vec::new())}
;
static TRACKING:RefCell<BTreeMap<(i64,i64),Rc<Device>>>=const{
RefCell::new(BTreeMap::new())}
;
}
const BUS: &str = "org.freedesktop.GeoClue2";
const CLIENT: &str = "org.freedesktop.GeoClue2.Client";
struct Device {
    bus: gio::DBusConnection,
    path: String,
}
impl Drop for Device {
    fn drop(&mut self) {
        let (bus, path) = (self.bus.clone(), self.path.clone());
        glib::spawn_future_local(async move {
            let _ = call(&bus, &path, CLIENT, "Stop", None).await;
        });
    }
}
async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<glib::Variant, String> {
    bus.call_future(
        Some(BUS),
        path,
        interface,
        method,
        args,
        None,
        gio::DBusCallFlags::NONE,
        10000,
    )
    .await
    .map_err(|e| format!("Device location unavailable: {e}"))
}
async fn properties(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
) -> Result<std::collections::HashMap<String, glib::Variant>, String> {
    let v = call(
        bus,
        path,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(interface,).to_variant()),
    )
    .await?;
    v.child_value(0)
        .get()
        .ok_or("The location provider returned invalid properties.".into())
}
fn unwrap(v: &glib::Variant) -> glib::Variant {
    v.as_variant().unwrap_or_else(|| v.clone())
}
impl Device {
    async fn start() -> Result<Self, String> {
        let bus = gio::bus_get_future(gio::BusType::System).await.map_err(|e| e.to_string())?;
        let result = call(
            &bus,
            "/org/freedesktop/GeoClue2/Manager",
            "org.freedesktop.GeoClue2.Manager",
            "GetClient",
            None,
        )
        .await?;
        let path = result.child_value(0).str().ok_or("No device location client.")?.to_owned();
        let client = Self { bus, path };
        for (key, value) in [
            ("DesktopId", "io.github.design_nexus.nyx.Pulse".to_variant()),
            ("RequestedAccuracyLevel", 8u32.to_variant()),
        ] {
            call(
                &client.bus,
                &client.path,
                "org.freedesktop.DBus.Properties",
                "Set",
                Some(&(CLIENT, key, value).to_variant()),
            )
            .await?;
        }
        call(&client.bus, &client.path, CLIENT, "Start", None).await?;
        Ok(client)
    }
    async fn fix(&self) -> Result<(f64, f64, f64), String> {
        for _ in 0..20 {
            let p = properties(&self.bus, &self.path, CLIENT).await?;
            let location = p
                .get("Location")
                .map(unwrap)
                .and_then(|v| v.str().map(str::to_owned))
                .unwrap_or_default();
            if location.is_empty() || location == "/" {
                glib::timeout_future(std::time::Duration::from_millis(500)).await;
                continue;
            }
            let p = properties(&self.bus, &location, "org.freedesktop.GeoClue2.Location").await?;
            let number = |key| {
                p.get(key)
                    .map(unwrap)
                    .and_then(|v| v.get::<f64>())
                    .ok_or_else(|| format!("Missing {key} in device location."))
            };
            let (lat, lon, accuracy) = (number("Latitude")?, number("Longitude")?, number("Accuracy")?);
            if !valid_fix(lat, lon, accuracy) {
                return Err("The device returned an invalid location.".into());
            }
            let timestamp = p
                .get("Timestamp")
                .map(unwrap)
                .and_then(|v| v.get::<(u64, u64)>())
                .map(|t| t.0)
                .unwrap_or(0);
            if timestamp == 0 || chrono::Utc::now().timestamp().saturating_sub(timestamp as i64) > 120 {
                return Err("The location provider's last fix is too old.".into());
            }
            return Ok((lat, lon, accuracy));
        }
        Err("The location provider has no current fix. Try again outdoors or use manual coordinates.".into())
    }
}
fn valid_fix(lat: f64, lon: f64, accuracy: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && accuracy.is_finite()
        && (-90.0..=90.).contains(&lat)
        && (-180.0..=180.).contains(&lon)
        && accuracy >= 0.
}
pub fn fill(scope: Scope, lat: gtk::Entry, lon: gtk::Entry, status: gtk::Label) {
    status.set_text("Waiting for device location permission and a current fix…");
    glib::spawn_future_local(async move {
        let result = async {
            let device = Device::start().await?;
            device.fix().await
        }
        .await;
        if !scope.valid() || lat.root().is_none() {
            return;
        }
        match result {
            Ok((a, b, accuracy)) => {
                lat.set_text(&format!("{a:.7}"));
                lon.set_text(&format!("{b:.7}"));
                status.set_text(&format!("Device location · accuracy about {accuracy:.0} m"));
            }
            Err(e) => status.set_text(&e),
        }
    });
}
pub fn update(v: &Value) {
    if v["@type"] != "updateActiveLiveLocationMessages" {
        return;
    }
    let messages = v["messages"].as_array().cloned().unwrap_or_default();
    let active = messages
        .iter()
        .filter_map(|m| Some((m["chat_id"].as_i64()?, m["id"].as_i64()?)))
        .collect::<Vec<_>>();
    TRACKING.with(|t| t.borrow_mut().retain(|k, _| active.contains(k)));
    ACTIVE.with(|v| *v.borrow_mut() = messages);
}
pub fn reset() {
    TRACKING.with(|v| v.borrow_mut().clear());
    ACTIVE.with(|v| v.borrow_mut().clear());
}
pub fn manage() {
    features::load("Live locations", |s, _d, b| async move {
        let messages = ACTIVE.with(|v| v.borrow().clone());
        if messages.is_empty() {
            note(&b, "No outgoing live locations reported by Telegram.");
        }
        for m in messages {
            let (chat, id) = (m["chat_id"].as_i64().unwrap_or(0), m["id"].as_i64().unwrap_or(0));
            let title = window::chat(chat).map(|c| c.title).unwrap_or_else(|| format!("Chat {chat}"));
            let tracking = TRACKING.with(|v| v.borrow().contains_key(&(chat, id)));
            note(
                &b,
                &format!(
                    "{title} · {}",
                    if tracking {
                        "automatic device updates"
                    } else {
                        "manual updates"
                    }
                ),
            );
            let scope = s.clone();
            button(&b, "Enable automatic device updates…", move || {
                if !scope.valid() {
                    return;
                }
                let scope = scope.clone();
                let (d, b) = nyx_ui::widgets::dialog("Automatic live location", 440);
                note(
                    &b,
                    "Your device location will be sent to this conversation approximately every 15 seconds while Pulse runs and this account stays active. This requires GeoClue and location permission. The existing sharing expiry is retained.",
                );
                button(&b, "Enable tracking", move || start(scope.clone(), chat, id));
                d.present();
            });
            let scope = s.clone();
            button(&b, "Disable automatic updates", move || {
                if !scope.valid() {
                    return;
                }
                TRACKING.with(|v| v.borrow_mut().remove(&(chat, id)));
                window::show_status(
                    "Automatic updates disabled. Your last position remains shared until expiry or Stop sharing.",
                );
            });
            let scope = s.clone();
            button(&b, "Stop sharing…", move || {
                if !scope.valid() {
                    return;
                }
                actions::operation(
                    "Stop sharing location?",
                    "The last position remains in the conversation.",
                    vec![],
                    "Stop sharing",
                    move |s, _| async move {
                        crate::groups::commit(
                            &s,
                            "editMessageLiveLocation",
                            json!({
                            "chat_id":chat,"message_id":id,"reply_markup":null,"location":null}
                            ),
                        )
                        .await?;
                        TRACKING.with(|v| v.borrow_mut().remove(&(chat, id)));
                        Ok(())
                    },
                );
            });
        }
        Ok(())
    });
}
fn start(scope: Scope, chat: i64, id: i64) {
    if TRACKING.with(|v| v.borrow().contains_key(&(chat, id))) {
        return;
    }
    glib::spawn_future_local(async move {
        let owner: RefCell<Option<Rc<Device>>> = RefCell::new(None);
        let result = async {
            let device = Rc::new(Device::start().await?);
            device.fix().await?;
            if !scope.valid() {
                return Err("Account changed.".into());
            }
            if TRACKING.with(|v| v.borrow().contains_key(&(chat, id))) {
                return Err("This location is already being tracked.".into());
            }
            TRACKING.with(|v| v.borrow_mut().insert((chat, id), device.clone()));
            *owner.borrow_mut() = Some(device.clone());
            window::show_status("Automatic location updates enabled.");
            loop {
                let current =
                    || scope.valid() && TRACKING.with(|v| v.borrow().get(&(chat, id)).is_some_and(|d| Rc::ptr_eq(d, &device)));
                if !current() {
                    break;
                }
                let (latitude, longitude, horizontal_accuracy) = device.fix().await?;
                if !current() {
                    break;
                }
                let message = scope
                    .call(
                        "getMessage",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                if message["content"]["@type"] != "messageLiveLocation"
                    || message["content"]["expires_in"].as_i64().unwrap_or(0) <= 0
                {
                    break;
                }
                if !current() {
                    break;
                }
                let mut live = message["content"]["location"].clone();
                live["location"] = json!({
                "@type":"location","latitude":latitude,"longitude":longitude,"horizontal_accuracy":horizontal_accuracy}
                );
                live["heading"] = json!(0);
                scope
                    .call(
                        "editMessageLiveLocation",
                        json!({
                        "chat_id":chat,"message_id":id,"reply_markup":null,"location":live}
                        ),
                    )
                    .await?;
                glib::timeout_future(std::time::Duration::from_secs(15)).await;
            }
            Ok::<(), String>(())
        }
        .await;
        if let Some(owner) = owner.borrow().as_ref() {
            TRACKING.with(|v| {
                let mut v = v.borrow_mut();
                if v.get(&(chat, id)).is_some_and(|d| Rc::ptr_eq(d, owner)) {
                    v.remove(&(chat, id));
                }
            });
        }
        if let Err(e) = result {
            scope.error(&format!(
                "Automatic location updates stopped. Last position remains shared until expiry. {e}"
            ));
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_location_fix_validation() {
        assert!(valid_fix(1., 2., 25.));
        assert!(!valid_fix(f64::NAN, 2., 25.));
        assert!(!valid_fix(1., 200., 25.));
        assert!(!valid_fix(1., 2., -1.));
    }
}

pub(crate) async fn device_fix()->Result<(f64,f64,f64),String>{let device=Device::start().await?;device.fix().await}
