//! New Serial Connection (UI 1.4, 2.2; D88): the port, from the ports of
//! this device (Refresh lists them again), the baud rate, data bits,
//! parity, stop bits and flow control. Connect opens the port in a new
//! session tab; the dialog offers the settings of the last Connect again
//! (this app run only: port names belong to one device, nothing is saved).

use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, Global, ScrollHandle, Task, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, WindowExt as _, h_flex, v_flex,
};
use shellpp_term_local::serial::{DataBits, FlowControl, Parity, PortInfo, SerialConfig, StopBits};

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
    config: SerialConfig,
    open: Option<List>,
    /// The baud list opens scrolled to the chosen rate.
    baud_scroll: ScrollHandle,
    _task: Option<Task<()>>,
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
        let mut form = Self {
            ports: None,
            config: last.unwrap_or_else(|| SerialConfig::new("")),
            open: None,
            baud_scroll: ScrollHandle::new(),
            _task: None,
        };
        form.refresh(window, cx);
        form
    }

    /// Lists the ports again, off the UI thread.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = cx.try_global::<SerialPorts>().map(|ports| ports.0.clone()) else {
            self.set_ports(Vec::new(), cx);
            return;
        };
        self.ports = None;
        cx.notify();
        let found = cx.background_spawn(async move { source.ports() });
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let ports = found.await;
            let _ = this.update(cx, |form, cx| form.set_ports(ports, cx));
        }));
    }

    /// The chosen port stays when it is still listed; else the first port.
    fn set_ports(&mut self, mut ports: Vec<PortInfo>, cx: &mut Context<Self>) {
        shellpp_term_local::serial::sort_ports(&mut ports);
        if !ports.iter().any(|port| port.name == self.config.port) {
            self.config.port = ports
                .first()
                .map(|port| port.name.clone())
                .unwrap_or_default();
        }
        self.ports = Some(ports);
        cx.notify();
    }

    /// What Connect opens; `None` without a port.
    pub fn config(&self) -> Option<SerialConfig> {
        let listed = self
            .ports
            .as_ref()
            .is_some_and(|ports| ports.iter().any(|port| port.name == self.config.port));
        listed.then(|| self.config.clone())
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

    fn port_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let value = match &self.ports {
            None => strings::SERIAL_LOOKING.to_owned(),
            Some(ports) if ports.is_empty() => strings::SERIAL_NO_PORTS.to_owned(),
            Some(_) => self.config.port.clone(),
        };
        let has_ports = self.ports.as_ref().is_some_and(|ports| !ports.is_empty());
        let trigger = widgets::select_trigger(
            "serial-port",
            None,
            value,
            self.open == Some(List::Port),
            cx,
        )
        .when(has_ports, |this| {
            this.on_click(cx.listener(|form, _: &ClickEvent, _, cx| form.toggle(List::Port, cx)))
        });
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
                        port.name == self.config.port,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |form, _: &ClickEvent, _, cx| {
                            let name = name.clone();
                            form.update(|config| config.port = name, cx);
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
                        .child(div().flex_1().min_w_0().child(trigger))
                        .child(refresh),
                )
                .children(list),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config.clone();
        v_flex()
            .id("serial-dialog")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(self.port_field(cx))
            .child(self.baud_field(cx))
            .child(self.choice(
                "serial-data",
                strings::SERIAL_DATA_BITS,
                &[
                    ("5", "5", DataBits::Five),
                    ("6", "6", DataBits::Six),
                    ("7", "7", DataBits::Seven),
                    ("8", "8", DataBits::Eight),
                ],
                config.data_bits,
                |config, value| config.data_bits = value,
                cx,
            ))
            .child(self.choice(
                "serial-parity",
                strings::SERIAL_PARITY,
                &[
                    ("none", strings::SERIAL_NONE, Parity::None),
                    ("odd", strings::SERIAL_ODD, Parity::Odd),
                    ("even", strings::SERIAL_EVEN, Parity::Even),
                ],
                config.parity,
                |config, value| config.parity = value,
                cx,
            ))
            .child(self.choice(
                "serial-stop",
                strings::SERIAL_STOP_BITS,
                &[("1", "1", StopBits::One), ("2", "2", StopBits::Two)],
                config.stop_bits,
                |config, value| config.stop_bits = value,
                cx,
            ))
            .child(self.choice(
                "serial-flow",
                strings::SERIAL_FLOW,
                &[
                    ("none", strings::SERIAL_NONE, FlowControl::None),
                    ("software", strings::SERIAL_XON_XOFF, FlowControl::Software),
                    ("hardware", strings::SERIAL_RTS_CTS, FlowControl::Hardware),
                ],
                config.flow,
                |config, value| config.flow = value,
                cx,
            ))
    }
}

/// Opens the dialog with the settings of the last Connect; `connect` gets
/// the settings when Connect (or Enter) is pressed with a port chosen.
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
