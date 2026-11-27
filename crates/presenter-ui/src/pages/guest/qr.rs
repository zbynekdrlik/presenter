//! `/poslech/qr` (#999 L4): a big QR code of the guest page for the operator
//! to show on a screen, with the address under it in plain text. The code is
//! made by the server for the address this page was opened through, so it is
//! only useful from the public address: a LAN address gets a warning.

use leptos::prelude::*;

#[component]
pub fn GuestQrPage() -> impl IntoView {
    crate::utils::window::document().set_title("Preklad kázne — QR");
    let qr = RwSignal::new(None::<crate::api::guest::GuestQr>);
    let failed = RwSignal::new(false);
    leptos::task::spawn_local(async move {
        match crate::api::guest::qr().await {
            Ok(code) => qr.set(Some(code)),
            Err(err) => {
                leptos::logging::log!("guest qr: not loaded: {err:?}");
                failed.set(true);
            }
        }
    });
    view! {
        <div class="guest-qr" data-role="guest-qr">
            <h1>"Translation · Übersetzung · Fordítás · Переклад"</h1>
            <Show when=move || failed.get() fallback=|| ()>
                <p class="guest-qr__warn">"QR kód sa nenačítal — obnovte stránku."</p>
            </Show>
            {move || qr.get().map(|code| view! {
                <div class="guest-qr__code" data-role="guest-qr-code" inner_html=code.svg.clone()></div>
                <p class="guest-qr__url" data-role="guest-qr-url">{code.url.clone()}</p>
                <Show when=move || code.local fallback=|| ()>
                    <p class="guest-qr__warn" data-role="guest-qr-local">
                        "Toto je miestna adresa — na mobilných dátach nefunguje. Otvorte túto stránku cez verejnú adresu (https://…/poslech/qr)."
                    </p>
                </Show>
            })}
        </div>
    }
}
