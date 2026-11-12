//! The home page `/` (#938): the way in on a phone. The most used surfaces
//! first as big tiles, the rest as full-width rows below, Slovak labels. The
//! order of the tiles was read from SNV's `GET /ui/usage` (90 days, 2026-11-12:
//! operator 440 presses + its tabs, study 302, the AI tab used from a phone,
//! tablet 69); read it again before reordering (`ui-usage.md`).

use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

use super::styles;

/// One link of the page.
struct Surface {
    href: &'static str,
    label: &'static str,
    hint: &'static str,
    /// Opens in a new tab (the settings page, as before).
    new_tab: bool,
}

const fn surface(href: &'static str, label: &'static str, hint: &'static str) -> Surface {
    Surface {
        href,
        label,
        hint,
        new_tab: false,
    }
}

/// The most used surfaces, in the order of their use.
const MOST_USED: [Surface; 4] = [
    surface(
        "/ui/operator",
        "Operátor",
        "Piesne, Biblia, časovače — ovládanie stien",
    ),
    surface("/ui/study", "Štúdium", "Biblia na čítanie a príprava kázne"),
    surface(
        "/ui/operator/ai",
        "AI asistent",
        "Operátor rovno na karte AI",
    ),
    surface(
        "/ui/tablet",
        "Tablet",
        "Piesne a kázeň na dotykovom tablete",
    ),
];

/// The other sections: a heading and its links.
const SECTIONS: [(&str, &[Surface]); 4] = [
    (
        "Stream",
        &[
            surface("/stream/stream", "Stream výstup (OBS)", "Grafika pre OBS"),
            surface("/ui/stream", "Stream editor", "Scény, menovky, písma"),
        ],
    ),
    (
        "Pódium a obrazovky",
        &[
            surface("/stage", "Pódiový výstup", "Text na obrazovkách pre pódium"),
            surface("/ui/camera", "Kameramani", "Obrazovka pre kameru"),
            surface("/stream/timer", "Časovač", "Časovač pre stream"),
        ],
    ),
    (
        "Tlmočenie",
        &[
            surface("/ui/sepkar", "Šepkár", "Tipy pre tlmočníka"),
            surface(
                "/ui/interpreter",
                "Tlmočník",
                "Obrazovka tlmočníka na tablete",
            ),
        ],
    ),
    (
        "Ďalšie",
        &[
            surface("/ui/bible", "Biblia", "Operátor na karte Biblia"),
            Surface {
                href: "/ui/settings",
                label: "Nastavenia",
                hint: "Zariadenia, integrácie, preklady",
                new_tab: true,
            },
        ],
    ),
];

fn link(surface: &Surface, class: &'static str) -> impl IntoView {
    let (target, rel) = if surface.new_tab {
        (Some("_blank"), Some("noopener"))
    } else {
        (None, None)
    };
    view! {
        <li>
            <a class=class href=surface.href target=target rel=rel>
                <span class="home__label">{surface.label}</span>
                <span class="home__hint">{surface.hint}</span>
            </a>
        </li>
    }
}

#[component]
fn HomeDocument() -> impl IntoView {
    view! {
        <html lang="sk">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>"Presenter"</title>
                <style>{styles::HOME}</style>
            </head>
            <body class="home">
                <main class="home__container">
                    <header class="home__header">
                        <h1>"Presenter"</h1>
                    </header>
                    <section class="home__section">
                        <h2>"Najčastejšie"</h2>
                        <ul class="home__tiles">
                            {MOST_USED.iter().map(|s| link(s, "home__tile")).collect_view()}
                        </ul>
                    </section>
                    {SECTIONS
                        .iter()
                        .map(|(title, links)| {
                            view! {
                                <section class="home__section">
                                    <h2>{*title}</h2>
                                    <ul class="home__links">
                                        {links.iter().map(|s| link(s, "home__link")).collect_view()}
                                    </ul>
                                </section>
                            }
                        })
                        .collect_view()}
                </main>
            </body>
        </html>
    }
}

pub async fn render_home_ui() -> anyhow::Result<Html<String>> {
    let owner = Owner::new_root(None);
    let html = owner.with(|| view! { <HomeDocument /> }.into_view().to_html());
    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
