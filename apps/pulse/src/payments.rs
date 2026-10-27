//! Reviewed native payment forms, shipping and tokenized/saved credentials.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note},
    window,
};
use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
type Done = Rc<dyn Fn(&str)>;
fn total(v: &Value) -> Result<i64, String> {
    v.as_array().ok_or("Invalid invoice prices.")?.iter().try_fold(0i64, |sum, p| {
        sum.checked_add(p["amount"].as_i64().ok_or("Invalid invoice amount.")?)
            .ok_or("Invoice amount exceeds supported range.".into())
    })
}
fn amount(value: i64, currency: &str) -> String {
    let exponents: Value = serde_json::from_str(include_str!("../data/currency-exponents.json")).unwrap_or(Value::Null);
    let digits = if currency == "XTR" {
        0
    } else if let Some(exp) = exponents[currency].as_u64().filter(|v| *v <= 6) {
        exp as u32
    } else {
        return format!("{value} {currency} (smallest units)");
    };
    if digits == 0 {
        format!("{value} {currency}")
    } else {
        let divisor = 10i64.pow(digits);
        format!("{}.{:0width$} {currency}", value / divisor, value.unsigned_abs() % divisor as u64, width = digits as usize)
    }
}
pub fn review(input: Value, done: Option<Done>) {
    features::load("Review payment", move |s, d, b| async move {
        let completed = Rc::new(Cell::new(false));
        let busy = Rc::new(Cell::new(false));
        let (c, busy_, callback) = (completed.clone(), busy.clone(), done.clone());
        d.connect_close_request(move |_| {
            if busy_.get() {
                return glib::Propagation::Stop;
            }
            if !c.replace(true) {
                if let Some(done) = &callback {
                    done("cancelled");
                }
            }
            glib::Propagation::Proceed
        });
        let form = s
            .call(
                "getPaymentForm",
                json!({
                "input_invoice":input,"theme":null}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, form["product_info"]["title"].as_str().unwrap_or("Invoice"));
        note(&b, form["product_info"]["description"].as_str().unwrap_or(""));
        let seller = s
            .call(
                "getUser",
                json!({
                "user_id":form["seller_bot_user_id"]}
                ),
            )
            .await
            .unwrap_or(Value::Null);
        if !alive(&s, &d) {
            return Ok(());
        }
        note(
            &b,
            &format!("Seller: {} {}", seller["first_name"].as_str().unwrap_or(""), seller["last_name"].as_str().unwrap_or("")),
        );
        let kind = form["type"]["@type"].as_str().unwrap_or("");
        if kind == "paymentFormTypeStars" || kind == "paymentFormTypeStarSubscription" {
            let price = if kind == "paymentFormTypeStars" {
                form["type"]["star_count"].as_i64()
            } else {
                form["type"]["pricing"]["star_count"].as_i64()
            }
            .ok_or("Invalid Star price.")?;
            let renewal = if kind == "paymentFormTypeStarSubscription" {
                format!("; renews every {} seconds", form["type"]["pricing"]["period"])
            } else {
                String::new()
            };
            let cost = format!("{price} Stars{renewal}");
            note(&b, &cost);
            let (scope, weak) = (s.clone(), d.downgrade());
            button(&b, "Pay with Stars…", move || {
                if scope.valid() {
                    confirm(
                        scope.clone(),
                        input.clone(),
                        form.clone(),
                        Value::Null,
                        String::new(),
                        String::new(),
                        0,
                        cost.clone(),
                        done.clone(),
                        completed.clone(),
                        busy.clone(),
                        weak.clone(),
                    );
                }
            });
            return Ok(());
        }
        if kind != "paymentFormTypeRegular" {
            return Err("Unknown payment form type. Update TDLib before paying.".into());
        }
        let invoice = &form["type"]["invoice"];
        let currency = invoice["currency"].as_str().ok_or("Missing invoice currency.")?.to_string();
        let base = total(&invoice["price_parts"])?;
        for part in invoice["price_parts"].as_array().into_iter().flatten() {
            note(
                &b,
                &format!(
                    "{}: {}",
                    part["label"].as_str().unwrap_or("Item"),
                    amount(part["amount"].as_i64().unwrap_or(0), &currency)
                ),
            );
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let saved = &form["type"]["saved_order_info"];
        let name = entry(&fields, "Name", saved["name"].as_str().unwrap_or(""), 255);
        let phone = entry(&fields, "Phone", saved["phone_number"].as_str().unwrap_or(""), 64);
        let email = entry(&fields, "Email", saved["email_address"].as_str().unwrap_or(""), 255);
        let country =
            entry(&fields, "Shipping country code", saved["shipping_address"]["country_code"].as_str().unwrap_or(""), 2);
        let state = entry(&fields, "State / region", saved["shipping_address"]["state"].as_str().unwrap_or(""), 255);
        let city = entry(&fields, "City", saved["shipping_address"]["city"].as_str().unwrap_or(""), 255);
        let street = entry(&fields, "Street", saved["shipping_address"]["street_line1"].as_str().unwrap_or(""), 255);
        let street2 = entry(&fields, "Address line 2", saved["shipping_address"]["street_line2"].as_str().unwrap_or(""), 255);
        let postal = entry(&fields, "Postal code", saved["shipping_address"]["postal_code"].as_str().unwrap_or(""), 32);
        for (w, show) in [
            (name.clone(), invoice["need_name"] == true),
            (phone.clone(), invoice["need_phone_number"] == true),
            (email.clone(), invoice["need_email_address"] == true),
        ] {
            w.set_visible(show);
            if let Some(label) = w.prev_sibling() {
                label.set_visible(show);
            }
        }
        for w in [&country, &state, &city, &street, &street2, &postal] {
            w.set_visible(invoice["need_shipping_address"] == true);
            if let Some(label) = w.prev_sibling() {
                label.set_visible(invoice["need_shipping_address"] == true);
            }
        }
        note(
            &fields,
            "Validate your order before selecting payment. Requested shipping/contact details are sent to the seller and, where required by the invoice, the provider.",
        );
        let validated: Rc<RefCell<Option<Value>>> = Default::default();
        let order_revision = Rc::new(Cell::new(0u64));
        for e in [&name, &phone, &email, &country, &state, &city, &street, &street2, &postal] {
            let (v, revision) = (validated.clone(), order_revision.clone());
            e.connect_changed(move |_| {
                revision.set(revision.get().wrapping_add(1));
                v.borrow_mut().take();
            });
        }
        let shipping = gtk::DropDown::from_strings(&["No shipping option"]);
        fields.append(&shipping);
        let status = note(&fields, "");
        let validate = gtk::Button::with_label("Validate order / shipping");
        fields.append(&validate);
        let (scope, weak) = (s.clone(), d.downgrade());
        let (v_, shipping_, status_) = (validated.clone(), shipping.clone(), status.clone());
        let validation_input = input.clone();
        let validation_revision = order_revision.clone();
        validate.connect_clicked(move|button|{
if !scope.valid(){
return}
button.set_sensitive(false);
let revision=validation_revision.get();
let revision_state=validation_revision.clone();
let info=json!({
"@type":"orderInfo","name":name.text().to_string(),"phone_number":phone.text().to_string(),"email_address":email.text().to_string(),"shipping_address":if gtk::prelude::WidgetExt::is_visible(&country){
json!({
"@type":"address","country_code":country.text().to_string().to_ascii_uppercase(),"state":state.text().to_string(),"city":city.text().to_string(),"street_line1":street.text().to_string(),"street_line2":street2.text().to_string(),"postal_code":postal.text().to_string()}
)}
else{
Value::Null}
}
);
let(s,input,v,shipping,status,button,weak)=(scope.clone(),validation_input.clone(),v_.clone(),shipping_.clone(),status_.clone(),button.clone(),weak.clone());
glib::spawn_future_local(async move{
let result=s.call("validateOrderInfo",json!({
"input_invoice":input,"order_info":info,"allow_save":false}
)).await;
button.set_sensitive(true);
if !s.valid()||weak.upgrade().is_none_or(|d|!d.is_visible()){
return}
if revision_state.get()!=revision{
status.set_text("Order details changed. Validate again.");
return}
match result{
Ok(data)=>{
let labels=data["shipping_options"].as_array().into_iter().flatten().map(|v|v["title"].as_str().unwrap_or("Shipping")).collect::<Vec<_>>();
let model=gtk::StringList::new(if labels.is_empty(){
&["No shipping required"]}
else{
&labels}
);
shipping.set_model(Some(&model));
shipping.set_selected(0);
*v.borrow_mut()=Some(data);
status.set_text("Order validated. Select shipping, then payment.");
}
,Err(e)=>{
v.borrow_mut().take();
status.set_text(&e);
}
}
}
);
}
);
        let tip = gtk::SpinButton::with_range(0., invoice["max_tip_amount"].as_f64().unwrap_or(0.).max(1.), 1.);
        tip.set_visible(invoice["max_tip_amount"].as_i64().unwrap_or(0) > 0);
        let tip_label = note(&fields, &format!("Optional tip in the invoice's smallest {currency} units"));
        tip_label.set_visible(invoice["max_tip_amount"].as_i64().unwrap_or(0) > 0);
        fields.append(&tip);
        let terms = check(&fields, "I authorize payment to the seller", false);
        for key in ["terms_of_service_url", "recurring_payment_terms_of_service_url"] {
            if let Some(url) = invoice[key].as_str().filter(|u| u.starts_with("https://")) {
                fields.append(&gtk::LinkButton::with_label(url, "Read payment terms"));
            }
        }
        let credentials: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));
        let provider = form["type"]["payment_provider"].clone();
        let (scope, credentials_, status_) = (s.clone(), credentials.clone(), status.clone());
        button(&fields, "Enter payment details securely…", move || {
            if !scope.valid() {
                return;
            }
            let (credentials, status) = (credentials_.clone(), status_.clone());
            if let Err(e) = crate::webapps::checkout(
                scope.clone(),
                &provider,
                Rc::new(move |token| {
                    *credentials.borrow_mut() = json!({
                    "@type":"inputCredentialsNew","data":if let Some(s)=token.as_str(){
                    s.to_string()}
                    else{
                    token.to_string()}
                    ,"allow_save":false}
                    );
                    status.set_text("Payment method ready. Review the final amount before paying.");
                }),
            ) {
                scope.error(&e);
            }
        });
        for saved in form["type"]["saved_credentials"].as_array().into_iter().flatten() {
            let id = saved["id"].clone();
            let title = format!("Use saved {}", saved["title"].as_str().unwrap_or("payment method"));
            let (scope, credentials, status) = (s.clone(), credentials.clone(), status.clone());
            button(&fields, &title, move || {
                if !scope.valid() {
                    return;
                }
                let (d, b) = nyx_ui::widgets::dialog("Unlock saved payment method", 440);
                let fields = nyx_ui::widgets::vbox(8);
                b.append(&fields);
                let password = gtk::PasswordEntry::new();
                password.set_show_peek_icon(true);
                note(&fields, "Telegram two-step verification password");
                fields.append(&password);
                let (id, credentials, status) = (id.clone(), credentials.clone(), status.clone());
                crate::groups::submit(&d, &b, &fields, scope.clone(), "Unlock", move |s| {
                    let (password, id, credentials, status) =
                        (password.text().to_string(), id.clone(), credentials.clone(), status.clone());
                    async move {
                        s.call(
                            "createTemporaryPassword",
                            json!({
                            "password":password,"valid_for":60}
                            ),
                        )
                        .await?;
                        *credentials.borrow_mut() = json!({
                        "@type":"inputCredentialsSaved","saved_credentials_id":id}
                        );
                        status.set_text("Saved payment method ready for 60 seconds.");
                        Ok(())
                    }
                });
                d.present();
            });
        }
        let (scope, weak) = (s.clone(), d.downgrade());
        button(&fields, "Review final payment…", move || {
            if !scope.valid() {
                return;
            }
            if !terms.is_active() {
                scope.error("Accept the seller's payment terms before continuing.");
                return;
            }
            let Some(order) = validated.borrow().clone() else {
                scope.error("Validate your order first.");
                return;
            };
            let selected = order["shipping_options"]
                .as_array()
                .and_then(|v| v.get(shipping.selected() as usize))
                .cloned()
                .unwrap_or(Value::Null);
            let tip = tip.value() as i64;
            let full = match total(&selected["price_parts"])
                .or_else(|_| if selected.is_null() { Ok(0) } else { Err(String::from("Choose shipping.")) })
                .and_then(|ship| {
                    base.checked_add(ship).and_then(|v| v.checked_add(tip)).ok_or_else(|| String::from("Total overflow."))
                }) {
                Ok(v) => v,
                Err(e) => {
                    scope.error(&e);
                    return;
                }
            };
            let credentials = credentials.borrow().clone();
            if credentials.is_null() {
                scope.error("Choose a payment method first.");
                return;
            }
            confirm(
                scope.clone(),
                input.clone(),
                form.clone(),
                credentials,
                order["order_info_id"].as_str().unwrap_or("").to_string(),
                selected["id"].as_str().unwrap_or("").to_string(),
                tip,
                amount(full, &currency),
                done.clone(),
                completed.clone(),
                busy.clone(),
                weak.clone(),
            );
        });
        Ok(())
    });
}
#[allow(clippy::too_many_arguments)]
fn confirm(
    scope: Scope,
    input: Value,
    form: Value,
    credentials: Value,
    order: String,
    shipping: String,
    tip: i64,
    cost: String,
    done: Option<Done>,
    completed: Rc<Cell<bool>>,
    busy: Rc<Cell<bool>>,
    parent: glib::WeakRef<gtk::Window>,
) {
    if !scope.valid() {
        return;
    }
    actions::operation(
        "Confirm payment",
        &format!("Pay {cost} to the reviewed seller? No credentials will be saved."),
        vec![],
        "Pay",
        move |s, _| {
            let (input, form, credentials, order, shipping, done, completed, busy, parent) = (
                input.clone(),
                form.clone(),
                credentials.clone(),
                order.clone(),
                shipping.clone(),
                done.clone(),
                completed.clone(),
                busy.clone(),
                parent.clone(),
            );
            async move {
                if busy.replace(true) {
                    return Err("A payment is already in progress.".into());
                }
                let result=async{
let fresh=s.call("getPaymentForm",json!({
"input_invoice":input,"theme":null}
)).await?;
if fresh["type"]["invoice"]!=form["type"]["invoice"]||fresh["seller_bot_user_id"]!=form["seller_bot_user_id"]||fresh["type"]["star_count"]!=form["type"]["star_count"]||fresh["type"]["pricing"]!=form["type"]["pricing"]{
return Err("Payment terms changed. Review the invoice again.".into())}
let result=crate::groups::commit(&s,"sendPaymentForm",json!({
"input_invoice":input,"payment_form_id":fresh["id"],"order_info_id":order,"shipping_option_id":shipping,"credentials":credentials,"tip_amount":tip}
)).await?;
if result["success"]==true{
if !completed.replace(true){
if let Some(done)=done{
done("paid");
}
}
window::show_status("Payment completed.");
Ok(())}
else if let Some(url)=result["verification_url"].as_str().filter(|u|u.starts_with("https://")){
let _=gio_open(url);
if let Some(done)=done{
if !completed.replace(true){
done("pending");
}
}
Ok(())}
else{
Err("Payment was not completed.".into())}
}
.await;
                busy.set(false);
                if result.is_ok() {
                    if let Some(d) = parent.upgrade() {
                        d.close();
                    }
                }
                result
            }
        },
    );
}
fn gio_open(url: &str) -> Result<(), glib::Error> {
    gtk::gio::AppInfo::launch_default_for_uri(url, gtk::gio::AppLaunchContext::NONE)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remainder_invoice_amounts() {
        assert_eq!(
            total(&json!([{
            "amount":100}
            ,{
            "amount":20}
            ]))
            .unwrap(),
            120
        );
        assert!(
            total(&json!([{
            "amount":i64::MAX}
            ,{
            "amount":1}
            ]))
            .is_err()
        );
        assert_eq!(amount(12345, "USD"), "123.45 USD");
        assert_eq!(amount(150, "JPY"), "150 JPY");
    }
}
