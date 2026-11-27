//! `/ui/guest-qr` (#999 L4): a big QR code of the guest page
//! (`https://<guest host>/`) for the operator to show on a screen, with the
//! address under it in plain text. Operator copy is Slovak; the heading is
//! for the guests, so it is in their languages.

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
            <h1>"Live translation · Übersetzung · Fordítás · Переклад"</h1>
            <Show when=move || failed.get() fallback=|| ()>
                <p class="guest-qr__warn">"QR kód sa nenačítal — obnovte stránku."</p>
            </Show>
            {move || {
                qr.get()
                    .map(|code| match (code.url, code.svg) {
                        (Some(url), Some(svg)) => {
                            view! {
                                <div class="guest-qr__code" data-role="guest-qr-code" inner_html=svg></div>
                                <p class="guest-qr__url" data-role="guest-qr-url">{url}</p>
                            }
                                .into_any()
                        }
                        _ => {
                            view! {
                                <p class="guest-qr__warn" data-role="guest-qr-unset">
                                    "Hosťovská adresa nie je na tomto počítači nastavená (PRESENTER_GUEST_HOST)."
                                </p>
                            }
                                .into_any()
                        }
                    })
            }}
        </div>
    }
}
