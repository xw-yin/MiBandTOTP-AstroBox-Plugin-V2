use astrobox_ng_wit::FutureReader;

use astrobox_ng_wit::astrobox::psys_host::ui_v3 as host_ui;
use astrobox_ng_wit::exports::astrobox::psys_plugin::{
    event_v3::{self, EventType},
    lifecycle,
};

pub mod logger;
pub mod ui;

struct MyPlugin;

impl event_v3::Guest for MyPlugin {
    #[allow(async_fn_in_trait)]
    fn on_event(event_type: EventType, event_payload: String) -> FutureReader<String> {
        let (writer, reader) = astrobox_ng_wit::wit_future::new::<String>(|| "".to_string());

        tracing::info!(
            "event_type: {:?}, event_payload: {}",
            event_type,
            event_payload
        );

        match event_type {
            EventType::InterconnectMessage => {
                astrobox_ng_wit::block_on(async move {
                    ui::handle_interconnect_message(event_payload).await;
                });
            }
            EventType::Timer => {
                astrobox_ng_wit::block_on(async move {
                    ui::handle_timer_event(event_payload).await;
                });
            }
            _ => {}
        }

        astrobox_ng_wit::spawn(async move {
            let _ = writer.write("".to_string()).await;
        });

        reader
    }

    fn on_ui_event_v3(
        event_id: String,
        event: host_ui::Event,
        event_payload: String,
    ) -> astrobox_ng_wit::FutureReader<String> {
        let (writer, reader) = astrobox_ng_wit::wit_future::new::<String>(|| "".to_string());

        tracing::info!(
            "on_ui_event_v3 received: event_id={}, event={:?}, payload={}",
            event_id,
            event,
            event_payload
        );

        astrobox_ng_wit::block_on(async move {
            ui::ui_event_processor(event, event_id, event_payload).await;
        });

        astrobox_ng_wit::spawn(async move {
            let _ = writer.write("".to_string()).await;
        });

        reader
    }

    fn on_ui_render(element_id: String) -> astrobox_ng_wit::FutureReader<()> {
        let (writer, reader) = astrobox_ng_wit::wit_future::new::<()>(|| ());

        tracing::info!("on_ui_render received: element_id={}", element_id);
        ui::render_main_ui(&element_id);

        astrobox_ng_wit::spawn(async move {
            let _ = writer.write(()).await;
        });

        reader
    }

    fn on_card_render(_card_id: String) -> astrobox_ng_wit::FutureReader<()> {
        let (writer, reader) = astrobox_ng_wit::wit_future::new::<()>(|| ());

        tracing::info!("on_card_render received");
        astrobox_ng_wit::spawn(async move {
            let _ = writer.write(()).await;
        });

        reader
    }
}

impl lifecycle::Guest for MyPlugin {
    #[allow(async_fn_in_trait)]
    fn on_load() -> () {
        logger::init();
        tracing::info!("MiBand TOTP AstroBox plugin loaded");
        astrobox_ng_wit::block_on(async {
            ui::register_interconnect_receivers("on_load").await;
        });
    }
}

astrobox_ng_wit::export!(MyPlugin);
