//! Multiline message composer model and GTK builder.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;

/// Pure state for the composer. The GTK view mirrors this model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposerModel {
    draft: String,
    sending: bool,
}

impl ComposerModel {
    pub fn draft(&self) -> &str {
        &self.draft
    }

    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.draft = draft.into();
    }

    pub fn clear(&mut self) {
        self.draft.clear();
    }

    pub fn set_sending(&mut self, sending: bool) {
        self.sending = sending;
    }

    pub fn is_sending(&self) -> bool {
        self.sending
    }

    pub fn send_affordance(&self) -> SendAffordance {
        SendAffordance {
            enabled: !self.sending && !self.draft.trim().is_empty(),
            label: if self.sending { "Sending…" } else { "Send" },
        }
    }

    pub fn take_draft(&mut self) -> Option<OutgoingMessageDraft> {
        if !self.send_affordance().enabled {
            return None;
        }
        Some(OutgoingMessageDraft {
            text: std::mem::take(&mut self.draft),
        })
    }
}

/// The state needed by a send button or an accessibility presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SendAffordance {
    pub enabled: bool,
    pub label: &'static str,
}

/// A backend-neutral message submission produced by the composer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutgoingMessageDraft {
    pub text: String,
}

/// GTK widgets and state for a multiline composer.
pub struct ComposerView {
    pub root: gtk::Box,
    pub input: gtk::TextView,
    pub send_button: gtk::Button,
    model: Rc<RefCell<ComposerModel>>,
}

impl ComposerView {
    pub fn model(&self) -> Rc<RefCell<ComposerModel>> {
        Rc::clone(&self.model)
    }

    pub fn draft(&self) -> String {
        let buffer = self.input.buffer();
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.text(&start, &end, false).to_string()
    }

    pub fn set_draft(&self, draft: &str) {
        self.input.buffer().set_text(draft);
        self.model.borrow_mut().set_draft(draft);
        self.update_send_button();
    }

    pub fn set_sending(&self, sending: bool) {
        self.model.borrow_mut().set_sending(sending);
        self.update_send_button();
    }

    pub fn connect_send<F>(&self, callback: F)
    where
        F: Fn(OutgoingMessageDraft) + 'static,
    {
        let input = self.input.clone();
        let model = Rc::clone(&self.model);
        self.send_button.connect_clicked(move |_| {
            let buffer = input.buffer();
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            let draft = buffer.text(&start, &end, false).to_string();
            model.borrow_mut().set_draft(draft);
            if let Some(draft) = model.borrow_mut().take_draft() {
                buffer.set_text("");
                callback(draft);
            }
        });
    }

    fn update_send_button(&self) {
        self.send_button
            .set_sensitive(self.model.borrow().send_affordance().enabled);
    }
}

/// Builds a responsive multiline composer.
///
/// The input is bounded to 160 pixels so a long draft remains usable. The
/// breakpoint bin changes the horizontal action row to a vertical row below
/// 520sp, keeping the send affordance reachable in narrow windows.
pub fn build_composer() -> ComposerView {
    let model = Rc::new(RefCell::new(ComposerModel::default()));
    let input = gtk::TextView::builder()
        .accepts_tab(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(6)
        .bottom_margin(6)
        .hexpand(true)
        .build();
    input.set_height_request(38);
    input.set_vexpand(false);

    let send_button = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text("Send message")
        .valign(gtk::Align::End)
        .sensitive(false)
        .build();
    send_button.add_css_class("suggested-action");

    let input_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .max_content_height(160)
        .propagate_natural_height(true)
        .hexpand(true)
        .child(&input)
        .build();
    input_scroll.add_css_class("card");

    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    layout.set_hexpand(true);
    layout.append(&input_scroll);
    layout.append(&send_button);

    let responsive = adw::BreakpointBin::builder().child(&layout).build();
    let narrow = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 520sp")
            .expect("composer breakpoint condition is valid"),
    );
    narrow.add_setter(
        &layout,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    narrow.add_setter(&layout, "spacing", Some(&6i32.to_value()));
    responsive.add_breakpoint(narrow);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_margin_start(12);
    root.set_margin_end(12);
    root.set_margin_top(8);
    root.set_margin_bottom(12);
    root.set_hexpand(true);
    root.append(&responsive);

    let model_for_change = Rc::clone(&model);
    let send_for_change = send_button.clone();
    input.buffer().connect_changed(move |buffer| {
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        model_for_change
            .borrow_mut()
            .set_draft(buffer.text(&start, &end, false));
        send_for_change.set_sensitive(model_for_change.borrow().send_affordance().enabled);
    });

    ComposerView {
        root,
        input,
        send_button,
        model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_requires_non_whitespace_and_not_sending() {
        let mut model = ComposerModel::default();
        assert!(!model.send_affordance().enabled);
        model.set_draft("  \n");
        assert!(!model.send_affordance().enabled);
        model.set_draft("hello\nworld");
        assert!(model.send_affordance().enabled);
        model.set_sending(true);
        assert!(!model.send_affordance().enabled);
    }

    #[test]
    fn taking_a_draft_preserves_multiline_text_and_clears_model() {
        let mut model = ComposerModel::default();
        model.set_draft("first line\nsecond line");
        assert_eq!(
            model.take_draft(),
            Some(OutgoingMessageDraft {
                text: "first line\nsecond line".to_owned()
            })
        );
        assert_eq!(model.draft(), "");
    }
}
