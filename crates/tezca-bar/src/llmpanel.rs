//! The SUPER+I lateral panel — a chat window docked to the edge of the screen.
//!
//! A layer-shell surface anchored right, with an exclusive zone, so the tiled
//! area reflows to make room instead of the panel covering your work. That is
//! the whole reason it is not an ordinary window: a floating chat box over a
//! terminal is something you move out of the way; a docked one is something you
//! type into while reading the terminal beside it.
//!
//! Toggling is [`gtk4::Application`] uniqueness rather than a signal. The
//! keybind runs `tezca-bar --llm-panel`; the first run opens the panel, and a
//! second run reaches the already-registered instance, whose `activate` closes
//! it. No new IPC, and no way to end up with two panels.

use crate::llm::{self, Chunk, LlmConfig, Message};
use crate::theme;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, EventControllerKey, Label,
    Orientation, ScrolledWindow, TextView, WrapMode,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;

/// Panel width, px. Wide enough for a code block at the panel's font size
/// without wrapping mid-identifier; narrow enough to leave a usable editor.
const WIDTH: i32 = 400;

/// Everything the panel needs to keep between turns.
struct Panel {
    cfg: LlmConfig,
    /// The conversation so far, which is also what gets re-sent each turn —
    /// Ollama's chat endpoint is stateless.
    history: RefCell<Vec<Message>>,
    /// Which model to send to. Starts at the configured one, or whatever is
    /// already resident, so the first message never has to load a second copy.
    model: RefCell<String>,
    /// True while a completion is streaming; a second send is refused rather
    /// than interleaved.
    busy: RefCell<bool>,
    /// Resolved once at open, so every turn goes to the server the probe found
    /// rather than re-probing (and possibly switching) mid-conversation.
    backend: std::cell::Cell<Option<llm::Backend>>,
    port: std::cell::Cell<u16>,
    messages: GtkBox,
    scroll: ScrolledWindow,
    input: TextView,
    status: Label,
    model_label: Label,
    send: Button,
}

/// Run the panel as its own application instance.
pub fn run(cfg: LlmConfig) -> glib::ExitCode {
    let app = Application::builder().application_id("dev.tezca.llm").build();
    app.connect_activate(move |app| {
        // Second launch: the keybind was pressed again, so this is a close.
        if let Some(win) = app.active_window() {
            win.close();
            return;
        }
        build(app, &cfg);
    });
    app.run_with_args(&["tezca-bar"])
}

fn build(app: &Application, cfg: &LlmConfig) {
    let Some(display) = gdk::Display::default() else { return };
    // The panel is a separate process from the bar, so it installs its own copy
    // of the theme stack; leaked deliberately, since it must outlive this call
    // for as long as the window is up.
    std::mem::forget(theme::CssStack::install(&display));

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("llm-panel");

    // --- header ------------------------------------------------------------
    let head = GtkBox::new(Orientation::Horizontal, 9);
    head.add_css_class("llm-head");
    let dot = GtkBox::new(Orientation::Horizontal, 0);
    dot.add_css_class("llm-dot");
    dot.set_valign(Align::Center);
    let model_label = Label::new(Some("…"));
    model_label.add_css_class("llm-model");
    model_label.set_xalign(0.0);
    model_label.set_hexpand(true);
    model_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let close = Button::with_label("\u{2715}");
    close.add_css_class("llm-icon");
    head.append(&dot);
    head.append(&model_label);
    head.append(&close);
    root.append(&head);

    // --- transcript --------------------------------------------------------
    let messages = GtkBox::new(Orientation::Vertical, 16);
    messages.add_css_class("llm-messages");
    let scroll = ScrolledWindow::new();
    scroll.set_child(Some(&messages));
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    root.append(&scroll);

    // --- composer ----------------------------------------------------------
    let foot = GtkBox::new(Orientation::Vertical, 9);
    foot.add_css_class("llm-foot");
    let input = TextView::new();
    input.add_css_class("llm-input");
    input.set_wrap_mode(WrapMode::WordChar);
    input.set_accepts_tab(false);
    let input_scroll = ScrolledWindow::new();
    input_scroll.set_child(Some(&input));
    input_scroll.set_max_content_height(120);
    input_scroll.set_propagate_natural_height(true);
    input_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let row = GtkBox::new(Orientation::Horizontal, 9);
    row.add_css_class("llm-composer");
    input_scroll.set_hexpand(true);
    row.append(&input_scroll);
    let send = Button::with_label("\u{2191}");
    send.add_css_class("llm-send");
    send.set_valign(Align::End);
    row.append(&send);
    foot.append(&row);

    let status = Label::new(Some("idle"));
    status.add_css_class("llm-status");
    status.set_xalign(0.0);
    foot.append(&status);
    root.append(&foot);

    let window = ApplicationWindow::builder().application(app).child(&root).build();
    window.add_css_class("tezca-llm");
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("tezca-llm"));
    // Anchored on three edges so it is a full-height column, and given an
    // exclusive zone so the compositor shrinks the tiling area rather than
    // letting the panel sit on top of it.
    window.set_anchor(Edge::Right, true);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_exclusive_zone(WIDTH);
    window.set_default_width(WIDTH);
    // A chat panel that cannot be typed into is a screenshot. `OnDemand` keeps
    // focus click-driven, so merely opening it does not steal your keyboard.
    window.set_keyboard_mode(KeyboardMode::OnDemand);

    let panel = Rc::new(Panel {
        cfg: cfg.clone(),
        history: RefCell::new(Vec::new()),
        model: RefCell::new(cfg.model.clone()),
        busy: RefCell::new(false),
        backend: std::cell::Cell::new(None),
        port: std::cell::Cell::new(0),
        messages,
        scroll,
        input,
        status,
        model_label,
        send,
    });

    // Which model to talk to. A configured name wins; otherwise adopt whatever
    // is already resident so the first message does not load a second copy of
    // a 40GB file alongside the one already in memory.
    {
        let p = panel.clone();
        let cfg = cfg.clone();
        glib::spawn_future_local(async move {
            let st = gtk4::gio::spawn_blocking(move || llm::poll_once(&cfg)).await;
            let Ok(st) = st else { return };
            if !st.up {
                p.model_label.set_text("Ollama is not running");
                p.status.set_text("start it with `ollama serve`");
                p.send.set_sensitive(false);
                return;
            }
            p.backend.set(st.backend);
            p.port.set(st.port);
            if p.model.borrow().is_empty() {
                let pick = st
                    .primary()
                    .map(|r| r.name.clone())
                    .or_else(|| st.available.first().cloned())
                    .unwrap_or_default();
                *p.model.borrow_mut() = pick;
            }
            p.refresh_header(&st);
        });
    }

    {
        let w = window.clone();
        close.connect_clicked(move |_| w.close());
    }
    {
        let p = panel.clone();
        panel.send.connect_clicked(move |_| p.submit());
    }
    // Enter sends, Shift+Enter is a newline — the convention every chat client
    // shares, and the reason the composer is a TextView rather than an Entry.
    // Escape closes.
    //
    // Both live on the *window* in the capture phase. A capture controller on
    // the TextView itself never saw Return: GTK routes a focused text widget's
    // keys through its input method first, and the IM consumes the keypress
    // before any controller on that widget runs. The window is above the IM in
    // the propagation path, so this is the one place that reliably sees it.
    {
        let p = panel.clone();
        let w = window.clone();
        let keys = EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, state| {
            use gdk::{Key, ModifierType};
            if key == Key::Escape {
                w.close();
                return glib::Propagation::Stop;
            }
            if matches!(key, Key::Return | Key::KP_Enter)
                && !state.contains(ModifierType::SHIFT_MASK)
            {
                p.submit();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    window.present();
    panel.input.grab_focus();
}

impl Panel {
    /// Header line: model, accelerator, and where it is loaded.
    fn refresh_header(&self, st: &llm::Status) {
        let model = self.model.borrow().clone();
        let detail = st
            .resident
            .iter()
            .find(|r| r.name == model)
            .map(|r| {
                // Each part is included only when the server actually reports
                // it — llama.cpp publishes no offload split, so nothing stands
                // in for one.
                let mut parts: Vec<String> = Vec::new();
                let size = r.size_text();
                if !size.is_empty() {
                    parts.push(size);
                }
                match (r.accel(), r.offload_pct()) {
                    (Some("split"), Some(p)) => parts.push(format!("{p}% on GPU")),
                    (Some(a), _) => parts.push(a.to_string()),
                    (None, _) => {}
                }
                // The context window is the number that decides whether a long
                // file will fit, so it belongs beside the size, not buried.
                if r.context > 0 {
                    parts.push(format!("{} ctx", llm::ctx_short(r.context)));
                }
                parts.join(" · ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "not loaded".to_string());
        self.model_label.set_text(&if model.is_empty() { "no model".to_string() } else { model });
        self.model_label.set_tooltip_text(Some(&detail));
        self.status.set_text(&detail);
    }

    /// Send whatever is in the composer.
    fn submit(self: &Rc<Self>) {
        if *self.busy.borrow() {
            return;
        }
        let buf = self.input.buffer();
        let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).trim().to_string();
        if text.is_empty() {
            return;
        }
        let model = self.model.borrow().clone();
        if model.is_empty() {
            self.status.set_text("no model to send to");
            return;
        }
        buf.set_text("");

        self.history.borrow_mut().push(Message { role: "user".into(), content: text.clone() });
        self.append_bubble("you", &text, true);

        // The reply bubble is created empty and filled as tokens arrive, so the
        // transcript grows in place rather than appearing all at once.
        let reply = self.append_bubble(short_name(&model), "", false);
        *self.busy.borrow_mut() = true;
        self.send.set_sensitive(false);
        self.status.set_text("generating…");

        let (tx, rx) = async_channel::unbounded::<Chunk>();
        let (backend, port) = match (self.backend.get(), self.port.get()) {
            (Some(b), p) => (b, p),
            _ => {
                self.status.set_text("no local model server");
                return;
            }
        };
        llm::stream(&self.cfg, backend, port, model, self.history.borrow().clone(), tx);

        let me = self.clone();
        glib::spawn_future_local(async move {
            let mut acc = String::new();
            let mut thinking = 0usize;
            while let Ok(chunk) = rx.recv().await {
                match chunk {
                    Chunk::Token(t) => {
                        acc.push_str(&t);
                        reply.set_text(&acc);
                        me.scroll_to_end();
                    }
                    // A reasoning model can think for many seconds before its
                    // first answer token. Showing the scratchpad's *size* keeps
                    // the panel visibly alive without passing a monologue off
                    // as the reply — the answer replaces it the moment it starts.
                    Chunk::Reasoning(t) => {
                        thinking += t.chars().count();
                        if acc.is_empty() {
                            me.status.set_text(&format!("thinking… {thinking} chars"));
                        }
                    }
                    Chunk::Done { tps, tokens } => {
                        me.history
                            .borrow_mut()
                            .push(Message { role: "assistant".into(), content: acc.clone() });
                        me.status.set_text(&format!("{tokens} tok · {tps:.0} tok/s"));
                        break;
                    }
                    Chunk::Error(e) => {
                        // Leave the partial reply in place: half an answer plus
                        // the reason it stopped is more use than either alone.
                        me.status.set_text(&format!("failed — {e}"));
                        break;
                    }
                }
            }
            *me.busy.borrow_mut() = false;
            me.send.set_sensitive(true);
            me.scroll_to_end();
        });
    }

    /// Add one turn to the transcript; returns the label its text lives in.
    fn append_bubble(&self, who: &str, text: &str, is_user: bool) -> Label {
        let b = GtkBox::new(Orientation::Vertical, 5);
        b.add_css_class(if is_user { "llm-user" } else { "llm-ai" });
        let name = Label::new(Some(who));
        name.add_css_class("llm-who");
        name.set_xalign(0.0);
        let body = Label::new(Some(text));
        body.add_css_class("llm-body");
        body.set_xalign(0.0);
        body.set_wrap(true);
        body.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        body.set_selectable(true);
        b.append(&name);
        b.append(&body);
        self.messages.append(&b);
        self.scroll_to_end();
        body
    }

    /// Keep the newest turn in view as it grows.
    fn scroll_to_end(&self) {
        let adj = self.scroll.vadjustment();
        // Deferred: the label has not been re-measured yet at the moment a
        // token lands, so scrolling now would stop one line short every time.
        glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
    }
}

/// `llama3.1:70b` → `llama3.1` — the tag is noise once it is in the header.
fn short_name(model: &str) -> &str {
    model.split(':').next().unwrap_or(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_name_drops_the_tag() {
        assert_eq!(short_name("llama3.1:70b"), "llama3.1");
        assert_eq!(short_name("qwen2.5-coder"), "qwen2.5-coder");
        assert_eq!(short_name(""), "");
    }
}
