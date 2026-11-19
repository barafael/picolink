//! What every page shares: the title, the status pill, and a button to connect until a device identified itself.

use std::convert::Infallible;

use leptos::{prelude::*, task::spawn_local};

use crate::{Client, Connection};

/// Shows `children` while a device is connected and identified. `session` is the page's, see [`Connection`].
#[component]
pub fn Page<S, F>(title: &'static str, connection: Connection, session: S, children: ChildrenFn) -> impl IntoView
where
    S: Fn(Client) -> F + Copy + Send + Sync + 'static,
    F: Future<Output = Result<Infallible, String>> + 'static,
{
    connection.connect_automatically(session);

    view! {
        <header>
            <h1>{title}</h1>
            <span class=move || format!("pill {}", connection.status().0)>{move || connection.status().1}</span>
        </header>
        <main>
            <Show
                when=move || connection.serial_number().is_some()
                fallback=move || {
                    view! {
                        <p class="hint">
                            <button
                                on:click=move |_| spawn_local(connection.connect_picked(session))
                                disabled=move || connection.is_connected()
                            >
                                "Connect…"
                            </button>
                        </p>
                    }
                }
            >
                {children()}
            </Show>
        </main>
    }
}
