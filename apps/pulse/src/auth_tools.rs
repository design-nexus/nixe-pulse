//! Additional native authentication steps and two-step verification recovery.
use crate::{
    actions::Scope,
    features::{self, alive, button, check},
    groups::{entry, note, submit},
};
use gtk::prelude::*;
use serde_json::{Value, json};
pub fn update(v: &Value) {
    if v["@type"] != "updateAuthorizationState" {
        return;
    }
    match v["authorization_state"]["@type"].as_str() {
        Some("authorizationStateWaitEmailAddress") => email(false),
        Some("authorizationStateWaitEmailCode") => email(true),
        Some("authorizationStateWaitPremiumPurchase") => {
            let (d, b) = nyx_ui::widgets::dialog("Telegram login requirement", 440);
            crate::dialog_dismissal::add(&d, &b, "Cancel");
            note(
                &b,
                "Telegram requires a mobile-store Premium purchase before this phone number can log in. Pulse cannot perform an official mobile-store purchase.",
            );
            if let Some(email) = v["authorization_state"]["support_email_address"].as_str() {
                note(&b, email);
            }
            d.present();
        }
        _ => {}
    }
}
pub fn registration() {
    features::load("Register Telegram account", |s, d, b| async move {
        let state = s.call("getAuthorizationState", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        note(
            &fields,
            state["terms_of_service"]["text"]["text"]
                .as_str()
                .unwrap_or("Review Telegram's terms before creating your account."),
        );
        let first = entry(&fields, "First name", "", 64);
        let last = entry(&fields, "Last name", "", 64);
        let terms = check(
            &fields,
            "I agree to Telegram's terms and meet the minimum age requirement",
            false,
        );
        let silent = check(&fields, "Do not notify my contacts that I joined", true);
        submit(&d, &b, &fields, s, "Create account", move |s| {
            let (first, last, terms, silent) = (
                first.text().trim().to_string(),
                last.text().trim().to_string(),
                terms.is_active(),
                silent.is_active(),
            );
            async move {
                if !terms || first.is_empty() {
                    return Err("Enter your name and accept the terms.".into());
                }
                crate::groups::commit(
                    &s,
                    "registerUser",
                    json!({
                    "first_name":first,"last_name":last,"disable_notification":silent}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
fn email(code: bool) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog(if code { "Email verification" } else { "Login email" }, 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let input = entry(
        &fields,
        if code { "Email verification code" } else { "Email address" },
        "",
        255,
    );
    submit(&d, &b, &fields, s, "Continue", move |s| {
        let input = input.text().trim().to_string();
        async move {
            if input.is_empty() {
                return Err("Enter the requested value.".into());
            }
            if code {
                s.call(
                    "checkAuthenticationEmailCode",
                    json!({
                    "code":{
                    "@type":"emailAddressAuthenticationCode","code":input}
                    }
                    ),
                )
                .await?;
            } else {
                s.call(
                    "setAuthenticationEmailAddress",
                    json!({
                    "email_address":input}
                    ),
                )
                .await?;
            }
            Ok(())
        }
    });
    d.present();
}
pub fn recover_login() {
    let Some(s) = Scope::current() else { return };
    features::load("Recover Telegram password", move |_s, d, b| async move {
        s.call("requestAuthenticationPasswordRecovery", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        note(
            &fields,
            "A recovery code was sent to your recovery email. Choose a new password; an empty new password removes two-step verification.",
        );
        let code = entry(&fields, "Recovery code", "", 64);
        let password = gtk::PasswordEntry::new();
        password.set_show_peek_icon(true);
        note(&fields, "New password");
        fields.append(&password);
        let hint = entry(&fields, "Hint", "", 128);
        submit(&d, &b, &fields, s, "Recover password", move |s| {
            let (code, password, hint) = (
                code.text().trim().to_string(),
                password.text().to_string(),
                hint.text().to_string(),
            );
            async move {
                if code.is_empty() {
                    return Err("Enter the recovery code.".into());
                }
                s.call(
                    "recoverAuthenticationPassword",
                    json!({
                    "recovery_code":code,"new_password":password,"new_hint":hint}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn security() {
    features::load("Two-step verification", |s, d, b| async move {
        let state = s.call("getPasswordState", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(
            &b,
            if state["has_password"] == true {
                "Two-step verification is enabled."
            } else {
                "Two-step verification is disabled."
            },
        );
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        note(&fields, "Current password (empty if none)");
        let old = gtk::PasswordEntry::new();
        old.set_show_peek_icon(true);
        fields.append(&old);
        note(&fields, "New password");
        let new = gtk::PasswordEntry::new();
        new.set_show_peek_icon(true);
        fields.append(&new);
        let remove = check(&fields, "Remove two-step verification", false);
        let hint = entry(&fields, "Password hint", "", 128);
        let email = entry(&fields, "New recovery email (leave blank to preserve)", "", 255);
        submit(&d, &b, &fields, s, "Save verification settings", move |s| {
            let (old, new, remove, hint, email) = (
                old.text().to_string(),
                new.text().to_string(),
                remove.is_active(),
                hint.text().to_string(),
                email.text().trim().to_string(),
            );
            async move {
                if new.is_empty() && !remove {
                    return Err("Enter a new password or explicitly choose removal.".into());
                }
                let state = s
                    .call(
                        "setPassword",
                        json!({
                        "old_password":old,"new_password":if remove{
                        "".to_string()}
                        else{
                        new}
                        ,"new_hint":hint,"set_recovery_email_address":!email.is_empty(),"new_recovery_email_address":email}
                        ),
                    )
                    .await?;
                if !state["recovery_email_address_code_info"].is_null() {
                    verify_email();
                }
                Ok(())
            }
        });
        button(&b, "Verify recovery email code", verify_email);
        button(&b, "Resend recovery email code", || {
            crate::actions::request(0, "resendRecoveryEmailAddressCode", json!({}))
        });
        Ok(())
    });
}
fn verify_email() {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Verify recovery email", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let code = entry(&fields, "Verification code", "", 64);
    submit(&d, &b, &fields, s, "Verify", move |s| {
        let code = code.text().trim().to_string();
        async move {
            s.call(
                "checkRecoveryEmailAddressCode",
                json!({
                "code":code}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
