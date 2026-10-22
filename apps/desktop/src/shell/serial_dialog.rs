//! New Serial Connection (UI 1.4, 2.2; D88, D95): the port, typed or chosen
//! from the ports of this device (Refresh lists them again), the baud rate,
//! data bits, parity, stop bits and flow control. Connect opens the port in
//! a new session tab; the dialog offers the settings of the last Connect
//! again, also after a restart: they are settings of this device
//! (`serial.*`, data-model 16.1), never imported, since port names belong
//! to one device.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, Global, ScrollHandle, Subscription, Task, Window,
    div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, WindowExt as _, h_flex, v_flex,
};
use serde_json::{Value, json};
use shellpp_core::service::dto::SettingsValues;
use shellpp_term_local::serial::{
    DataBits, FlowControl, Parity, PortInfo, SerialConfig, StopBits, same_port,
};

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::sessions::SerialBackend;
use crate::theme::metrics;

const DIALOG_WIDTH: gpui::Pixels = px(440.);

/// The baud rates offered, slowest first.
pub const BAUD_RATES: [u32; 12] = [
    300, 1200, 2400, 4800, 9600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800, 921_600,
];

/// The options of the segmented choices: the id suffix, which is also the
/// value of the setting (`serial.data_bits`, …), the label, the value.
const DATA_BITS: [(&str, &str, DataBits); 4] = [
    ("5", "5", DataBits::Five),
    ("6", "6", DataBits::Six),
    ("7", "7", DataBits::Seven),
    ("8", "8", DataBits::Eight),
];
const PARITIES: [(&str, &str, Parity); 3] = [
    ("none", strings::SERIAL_NONE, Parity::None),
    ("odd", strings::SERIAL_ODD, Parity::Odd),
    ("even", strings::SERIAL_EVEN, Parity::Even),
];
const STOP_BITS: [(&str, &str, StopBits); 2] =
    [("1", "1", StopBits::One), ("2", "2", StopBits::Two)];
const FLOWS: [(&str, &str, FlowControl); 3] = [
    ("none", strings::SERIAL_NONE, FlowControl::None),
    ("software", strings::SERIAL_XON_XOFF, FlowControl::Software),
    ("hardware", strings::SERIAL_RTS_CTS, FlowControl::Hardware),
];

/// The longest port name, in bytes (the `serial.port` setting).
pub const MAX_PORT_BYTES: usize = 256;

/// The hint in an empty port field: a port name of this system.
const PORT_PLACEHOLDER: &str = if cfg!(windows) {
    "COM3"
} else {
    "/dev/ttyUSB0"
};

/// Where the dialog finds the ports: the session layer's backend (tests
/// install a fake).
#[derive(Clone)]
pub struct SerialPorts(pub Arc<dyn SerialBackend>);

impl Global for SerialPorts {}

impl std::fmt::Debug for SerialPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SerialPorts")
    }
}

fn named<T: Copy>(options: &[(&str, &str, T)], name: Option<&str>) -> Option<T> {
    options
        .iter()
        .find(|&&(suffix, _, _)| Some(suffix) == name)
        .map(|&(_, _, value)| value)
}

fn name_of<T: Copy + PartialEq>(options: &[(&'static str, &str, T)], value: T) -> &'static str {
    options
        .iter()
        .find(|&&(_, _, option)| option == value)
        .map_or("", |&(suffix, _, _)| suffix)
}

/// The settings of the last Connect (D95); `None` before the first one. A
/// value the settings do not hold keeps its default.
pub fn saved(values: &SettingsValues) -> Option<SerialConfig> {
    let text = |key: &str| values.get(key).and_then(Value::as_str);
    let mut config = SerialConfig::new(text("serial.port")?);
    if let Some(baud) = values
        .get("serial.baud")
        .and_then(Value::as_u64)
        .and_then(|baud| u32::try_from(baud).ok())
    {
        config.baud = baud;
    }
    config.data_bits = named(&DATA_BITS, text("serial.data_bits")).unwrap_or_default();
    config.parity = named(&PARITIES, text("serial.parity")).unwrap_or_default();
    config.stop_bits = named(&STOP_BITS, text("serial.stop_bits")).unwrap_or_default();
    config.flow = named(&FLOWS, text("serial.flow")).unwrap_or_default();
    Some(config)
}

/// What [`saved`] reads back: the settings of a Connect.
pub fn settings_of(config: &SerialConfig) -> BTreeMap<String, Value> {
    [
        ("serial.port", json!(config.port)),
        ("serial.baud", json!(config.baud)),
        (
            "serial.data_bits",
            json!(name_of(&DATA_BITS, config.data_bits)),
        ),
        ("serial.parity", json!(name_of(&PARITIES, config.parity))),
        (
            "serial.stop_bits",
            json!(name_of(&STOP_BITS, config.stop_bits)),
        ),
        ("serial.flow", json!(name_of(&FLOWS, config.flow))),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

/// The open dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    Port,
    Baud,
}

/// The form of the dialog.
pub struct SerialForm {
    /// `None` while the ports are being listed.
    ports: Option<Vec<PortInfo>>,
    /// The settings; `port` is the field's text without the spaces around.
    config: SerialConfig,
    /// The port name: typed, or chosen from the list of found ports.
    port: Entity<InputState>,
    open: Option<List>,
    /// The baud list opens scrolled to the chosen rate.
    baud_scroll: ScrollHandle,
    _task: Option<Task<()>>,
    _typed: Subscription,
}

impl std::fmt::Debug for SerialForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialForm")
            .field("listed", &self.ports.as_ref().map(Vec::len))
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl SerialForm {
    fn new(last: Option<SerialConfig>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = last.unwrap_or_else(|| SerialConfig::new(""));
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(config.port.clone())
                .placeholder(PORT_PLACEHOLDER)
        });
        let typed = cx.subscribe(&port, |form: &mut Self, port, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                form.config.port = port.read(cx).value().trim().to_owned();
                cx.notify();
            }
        });
        let mut form = Self {
            ports: None,
            config,
            port,
            open: None,
            baud_scroll: ScrollHandle::new(),
            _task: None,
            _typed: typed,
        };
        form.refresh(window, cx);
        form
    }

    /// Lists the ports again, off the UI thread.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = cx.try_global::<SerialPorts>().map(|ports| ports.0.clone()) else {
            self.set_ports(Vec::new(), window, cx);
            return;
        };
        self.ports = None;
        cx.notify();
        let found = cx.background_spawn(async move { source.ports() });
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let ports = found.await;
            let _ = this.update_in(cx, |form, window, cx| form.set_ports(ports, window, cx));
        }));
    }

    /// An empty field takes the first port; a name in it stays, listed or
    /// not (a port saved or typed that is not plugged in now, D95).
    fn set_ports(&mut self, mut ports: Vec<PortInfo>, window: &mut Window, cx: &mut Context<Self>) {
        shellpp_term_local::serial::sort_ports(&mut ports);
        if self.config.port.is_empty()
            && let Some(first) = ports.first()
        {
            self.set_port(first.name.clone(), window, cx);
        }
        self.ports = Some(ports);
        cx.notify();
    }

    /// Puts `name` in the port field.
    fn set_port(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.port
            .update(cx, |port, cx| port.set_value(name.clone(), window, cx));
        self.config.port = name;
    }

    /// What Connect opens; `None` while the port field is empty or holds a
    /// name longer than a port name can be.
    pub fn config(&self) -> Option<SerialConfig> {
        (!self.config.port.is_empty() && self.config.port.len() <= MAX_PORT_BYTES)
            .then(|| self.config.clone())
    }

    /// The port in the field is not among the ports found (D95).
    fn not_found(&self) -> bool {
        !self.config.port.is_empty()
            && self.ports.as_ref().is_some_and(|ports| {
                !ports
                    .iter()
                    .any(|port| same_port(&port.name, &self.config.port))
            })
    }

    fn toggle(&mut self, list: List, cx: &mut Context<Self>) {
        self.open = (self.open != Some(list)).then_some(list);
        if self.open == Some(List::Baud)
            && let Some(index) = BAUD_RATES.iter().position(|&baud| baud == self.config.baud)
        {
            self.baud_scroll.scroll_to_item(index);
        }
        cx.notify();
    }

    fn update(&mut self, change: impl FnOnce(&mut SerialConfig), cx: &mut Context<Self>) {
        change(&mut self.config);
        self.open = None;
        cx.notify();
    }

    /// Under the port field: a name too long, the listing, that the port in
    /// the field is not found now, or no ports.
    fn port_status(&self, cx: &App) -> Option<gpui::AnyElement> {
        let theme = cx.theme();
        let (id, text, color) = match &self.ports {
            _ if self.config.port.len() > MAX_PORT_BYTES => (
                "serial-port-too-long",
                strings::SERIAL_TOO_LONG,
                theme.danger,
            ),
            None => (
                "serial-port-looking",
                strings::SERIAL_LOOKING,
                theme.muted_foreground,
            ),
            Some(_) if self.not_found() => (
                "serial-port-not-found",
                strings::SERIAL_NOT_FOUND,
                theme.warning,
            ),
            Some(ports) if ports.is_empty() => (
                "serial-port-none",
                strings::SERIAL_NO_PORTS,
                theme.muted_foreground,
            ),
            Some(_) => return None,
        };
        Some(
            div()
                .id(id)
                .test_support()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(color)
                .child(text)
                .into_any_element(),
        )
    }

    fn port_field(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_ports = self.ports.as_ref().is_some_and(|ports| !ports.is_empty());
        let choose = has_ports.then(|| {
            widgets::icon_button(
                "serial-port-choose",
                if self.open == Some(List::Port) {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                },
                cx,
            )
            .on_click(cx.listener(|form, _: &ClickEvent, _, cx| form.toggle(List::Port, cx)))
        });
        let input = widgets::text_input_with(
            Input::new(&self.port).when_some(choose, |input, choose| input.suffix(choose)),
            &self.port,
            "serial-port",
            false,
            window,
            cx,
        );
        let refresh = widgets::icon_button("serial-refresh", IconName::RefreshCw, cx)
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(strings::SERIAL_REFRESH)
                    .build(window, cx)
            })
            .on_click(cx.listener(|form, _: &ClickEvent, window, cx| form.refresh(window, cx)));
        let list = (self.open == Some(List::Port)).then(|| {
            widgets::dropdown_list("serial-port-list", cx)
                .max_h(widgets::PICKER_MAX_HEIGHT)
                .overflow_y_scroll()
                .children(self.ports.iter().flatten().map(|port| {
                    let name = port.name.clone();
                    widgets::dropdown_item(
                        format!("serial-port-{}", port.name),
                        None,
                        port.name.clone(),
                        port.description.clone().map(Into::into),
                        same_port(&port.name, &self.config.port),
                        cx,
                    )
                    .on_click(cx.listener(
                        move |form, _: &ClickEvent, window, cx| {
                            form.set_port(name.clone(), window, cx);
                            form.update(|_| {}, cx);
                        },
                    ))
                }))
        });
        widgets::field(
            strings::SERIAL_PORT,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(
                    h_flex()
                        .w_full()
                        .gap(metrics::SPACE_2)
                        .child(div().flex_1().min_w_0().child(input))
                        .child(refresh),
                )
                .children(list)
                .children(self.port_status(cx)),
            None,
            cx,
        )
    }

    fn baud_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let trigger = widgets::select_trigger(
            "serial-baud",
            None,
            self.config.baud.to_string(),
            self.open == Some(List::Baud),
            cx,
        )
        .on_click(cx.listener(|form, _: &ClickEvent, _, cx| form.toggle(List::Baud, cx)));
        let list = (self.open == Some(List::Baud)).then(|| {
            widgets::dropdown_list("serial-baud-list", cx)
                .max_h(widgets::PICKER_MAX_HEIGHT)
                .overflow_y_scroll()
                .track_scroll(&self.baud_scroll)
                .children(BAUD_RATES.iter().map(|&baud| {
                    widgets::dropdown_item(
                        format!("serial-baud-{baud}"),
                        None,
                        baud.to_string(),
                        None,
                        baud == self.config.baud,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |form, _: &ClickEvent, _, cx| {
                            form.update(|config| config.baud = baud, cx);
                        },
                    ))
                }))
        });
        widgets::field(
            strings::SERIAL_BAUD,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list),
            None,
            cx,
        )
    }

    /// A segmented choice of `options` (id suffix, label, value).
    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        label: &'static str,
        options: &[(&'static str, &'static str, T)],
        current: T,
        set: fn(&mut SerialConfig, T),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let segments = options.iter().map(|&(suffix, text, value)| {
            widgets::segment(format!("{id}-{suffix}"), text, current == value, cx)
                .on_click(cx.listener(move |form, _: &ClickEvent, _, cx| {
                    form.update(|config| set(config, value), cx);
                }))
                .into_any_element()
        });
        widgets::field(
            label,
            widgets::segments(segments.collect::<Vec<_>>()),
            None,
            cx,
        )
    }
}

impl Render for SerialForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config.clone();
        v_flex()
            .id("serial-dialog")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(self.port_field(window, cx))
            .child(self.baud_field(cx))
            .child(self.choice(
                "serial-data",
                strings::SERIAL_DATA_BITS,
                &DATA_BITS,
                config.data_bits,
                |config, value| config.data_bits = value,
                cx,
            ))
            .child(self.choice(
                "serial-parity",
                strings::SERIAL_PARITY,
                &PARITIES,
                config.parity,
                |config, value| config.parity = value,
                cx,
            ))
            .child(self.choice(
                "serial-stop",
                strings::SERIAL_STOP_BITS,
                &STOP_BITS,
                config.stop_bits,
                |config, value| config.stop_bits = value,
                cx,
            ))
            .child(self.choice(
                "serial-flow",
                strings::SERIAL_FLOW,
                &FLOWS,
                config.flow,
                |config, value| config.flow = value,
                cx,
            ))
    }
}

/// Opens the dialog with the settings of the last Connect; `connect` gets
/// the settings when Connect (or Enter) is pressed with a port in the field.
pub fn open(
    last: Option<SerialConfig>,
    window: &mut Window,
    cx: &mut App,
    connect: impl Fn(SerialConfig, &mut Window, &mut App) + 'static,
) -> Entity<SerialForm> {
    let form = cx.new(|cx| SerialForm::new(last, window, cx));
    let shown = form.clone();
    let connect = Rc::new(connect);
    window.open_dialog(cx, move |dialog, _, cx| {
        let submit = {
            let form = shown.downgrade();
            let connect = connect.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let Some(config) = form.upgrade().and_then(|form| form.read(cx).config()) else {
                    return;
                };
                window.close_dialog(cx);
                connect(config, window, cx);
            })
        };
        let on_ok = submit.clone();
        let ready = shown.read(cx).config().is_some();
        dialog
            .w(DIALOG_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::SERIAL_TITLE),
            )
            .child(shown.clone())
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("serial-cancel", ControlKind::Neutral)
                            .label(strings::SERIAL_CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("serial-connect", ControlKind::Accent)
                            .label(strings::SERIAL_CONNECT)
                            .disabled(!ready)
                            .on_click(move |_, window, cx| submit(window, cx)),
                    ),
            )
    });
    form
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D95: the settings of a Connect read back as they were; a value the
    /// settings do not hold keeps its default; no port, no last Connect.
    #[test]
    fn the_last_connect_reads_back_from_the_settings() {
        let mut config = SerialConfig::new(r"\\.\COM12");
        config.baud = 115_200;
        config.data_bits = DataBits::Seven;
        config.parity = Parity::Odd;
        config.stop_bits = StopBits::Two;
        config.flow = FlowControl::Software;
        let values: SettingsValues = settings_of(&config).into_iter().collect();
        assert_eq!(values["serial.parity"], json!("odd"));
        assert_eq!(values["serial.flow"], json!("software"));
        assert_eq!(saved(&values), Some(config));

        let mut values = SettingsValues::new();
        assert_eq!(saved(&values), None);
        values.insert("serial.port".into(), Value::Null);
        assert_eq!(saved(&values), None);
        values.insert("serial.port".into(), json!("COM3"));
        values.insert("serial.parity".into(), json!("mark"));
        assert_eq!(saved(&values), Some(SerialConfig::new("COM3")));
    }
}
