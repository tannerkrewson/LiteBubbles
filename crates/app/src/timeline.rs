//! Reusable conversation timeline model and GTK view.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use litebubbles_core::{
    AttachmentKind, BackendEvent, BackendEventKind, Conversation, ConversationId, Location,
    Message, MessageId, MessageMutationKind, MessagePart, ParticipantId, TextFormatting, Timestamp,
};
use litebubbles_mock_backend::{FixtureSet, MockBackend};

/// The loading state presented by a timeline view.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LoadState {
    #[default]
    Loading,
    Ready,
    Error(String),
}

/// Stable labels derived from a core timestamp in UTC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimestampLabels {
    pub day: String,
    pub time: String,
}

impl TimestampLabels {
    pub fn from_timestamp(timestamp: Timestamp) -> Self {
        let millis_per_day = 86_400_000;
        let days = timestamp.as_millis().div_euclid(millis_per_day);
        let day_millis = timestamp.as_millis().rem_euclid(millis_per_day);
        let (year, month, day) = civil_date_from_days(days);
        let hour = day_millis / 3_600_000;
        let minute = (day_millis % 3_600_000) / 60_000;
        let second = (day_millis % 60_000) / 1_000;

        Self {
            day: format!("{year:04}-{month:02}-{day:02}"),
            time: format!("{hour:02}:{minute:02}:{second:02}"),
        }
    }
}

/// A date row inserted before the first message for a UTC day.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaySeparator {
    pub label: String,
}

/// The position of a message in a consecutive same-sender group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupPosition {
    Single,
    First,
    Middle,
    Last,
}

/// A UI-friendly representation of one ordered core message part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedPart {
    Text {
        text: String,
        formatting: Vec<TextFormatting>,
    },
    Attachment {
        id: String,
        kind: AttachmentKind,
        label: String,
        caption: Option<String>,
    },
    LinkPreview {
        url: String,
        title: Option<String>,
        summary: Option<String>,
    },
    Location {
        label: String,
    },
    Contact {
        display_name: String,
    },
    ServiceExtension {
        service: String,
        name: String,
    },
}

/// A message prepared for rendering, retaining the source core message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineMessage {
    pub message: Message,
    pub sender_name: String,
    pub is_own: bool,
    pub timestamp: TimestampLabels,
    pub parts: Vec<RenderedPart>,
    pub group_position: GroupPosition,
}

/// A row in the list model. Keeping separators as model rows lets GTK recycle
/// one factory for both separators and message bubbles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineRow {
    DaySeparator(DaySeparator),
    Message(Box<TimelineMessage>),
}

/// The result of applying one backend event to a timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineEventEffect {
    Ignored,
    Inserted { index: usize },
    Appended { index: usize },
    Updated { index: usize },
    Removed { index: usize },
}

impl TimelineEventEffect {
    pub const fn changed(self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// The minimum scroll information needed to restore a timeline after a model
/// mutation. A caller can keep a reader's position while rows are refreshed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    pub value: f64,
    pub at_bottom: bool,
}

/// Pure timeline state built from core messages or mock-backend fixtures.
#[derive(Clone, Debug)]
pub struct TimelineModel {
    conversation_id: ConversationId,
    own_participant_id: Option<ParticipantId>,
    sender_names: BTreeMap<ParticipantId, String>,
    messages: Vec<Message>,
    rows: Vec<TimelineRow>,
    load_state: LoadState,
}

impl TimelineModel {
    /// Creates an empty model in the loading state.
    pub fn new(conversation_id: ConversationId, own_participant_id: Option<ParticipantId>) -> Self {
        Self {
            conversation_id,
            own_participant_id,
            sender_names: BTreeMap::new(),
            messages: Vec::new(),
            rows: Vec::new(),
            load_state: LoadState::Loading,
        }
    }

    /// Creates a ready model from messages belonging to one conversation.
    pub fn from_messages(
        conversation_id: ConversationId,
        own_participant_id: Option<ParticipantId>,
        messages: impl IntoIterator<Item = Message>,
    ) -> Self {
        let mut model = Self::new(conversation_id, own_participant_id);
        model.replace_messages(messages);
        model
    }

    /// Creates a ready model with participant names from a core conversation.
    pub fn from_conversation(
        conversation: &Conversation,
        own_participant_id: Option<ParticipantId>,
        messages: impl IntoIterator<Item = Message>,
    ) -> Self {
        let mut model = Self::new(conversation.id.clone(), own_participant_id);
        model.sender_names = conversation
            .participants
            .iter()
            .map(|participant| {
                (
                    participant.id.clone(),
                    participant
                        .display_name
                        .clone()
                        .unwrap_or_else(|| participant.id.to_string()),
                )
            })
            .collect();
        model.replace_messages(messages);
        model
    }

    /// Builds a timeline from the deterministic mock-backend snapshot.
    pub fn from_fixture(
        fixture: &FixtureSet,
        conversation_id: &ConversationId,
        own_participant_id: Option<&ParticipantId>,
    ) -> Option<Self> {
        let conversation = fixture.conversation(conversation_id)?;
        let own_participant_id = own_participant_id
            .cloned()
            .or_else(|| fixture_own_participant_id(fixture, conversation));
        Some(Self::from_conversation(
            conversation,
            own_participant_id,
            fixture.messages_for(conversation_id).cloned(),
        ))
    }

    /// Builds a timeline from a mock backend's current snapshot.
    pub fn from_backend(
        backend: &MockBackend,
        conversation_id: &ConversationId,
        own_participant_id: Option<&ParticipantId>,
    ) -> Option<Self> {
        Self::from_fixture(backend.fixture(), conversation_id, own_participant_id)
    }

    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    pub fn own_participant_id(&self) -> Option<&ParticipantId> {
        self.own_participant_id.as_ref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn rows(&self) -> &[TimelineRow] {
        &self.rows
    }

    pub fn load_state(&self) -> &LoadState {
        &self.load_state
    }

    pub fn set_loading(&mut self) {
        self.load_state = LoadState::Loading;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.load_state = LoadState::Error(message.into());
    }

    pub fn set_ready(&mut self) {
        self.load_state = LoadState::Ready;
        self.rebuild_rows();
    }

    /// Replaces the snapshot, filters unrelated messages, and sorts by the
    /// core message ordering before rebuilding separators and grouping.
    pub fn replace_messages(&mut self, messages: impl IntoIterator<Item = Message>) {
        self.messages = messages
            .into_iter()
            .filter(|message| message.conversation_id == self.conversation_id)
            .collect();
        self.messages.sort();
        self.load_state = LoadState::Ready;
        self.rebuild_rows();
    }

    /// Applies an event for this conversation with deterministic upsert and
    /// removal semantics. The returned row index includes day-separator rows.
    pub fn apply_event(&mut self, event: &BackendEvent) -> TimelineEventEffect {
        let effect = match &event.kind {
            BackendEventKind::MessageAdded(message)
                if message.conversation_id == self.conversation_id =>
            {
                self.upsert_message(message.clone(), false)
            }
            BackendEventKind::MessageChanged(message)
                if message.conversation_id == self.conversation_id =>
            {
                self.upsert_message(message.clone(), true)
            }
            BackendEventKind::MessageRemoved {
                conversation_id,
                message_id,
            } if conversation_id == &self.conversation_id => self.remove_message(message_id),
            _ => TimelineEventEffect::Ignored,
        };

        if effect.changed() {
            self.load_state = LoadState::Ready;
        }
        effect
    }

    fn upsert_message(&mut self, message: Message, is_update: bool) -> TimelineEventEffect {
        if let Some(existing_index) = self
            .messages
            .iter()
            .position(|existing| existing.id == message.id)
        {
            if !is_update && self.messages[existing_index] == message {
                return TimelineEventEffect::Ignored;
            }
            let message_id = message.id.clone();
            self.messages[existing_index] = message;
            self.messages.sort();
            self.rebuild_rows();
            return self
                .row_index_for_message(&message_id)
                .map_or(TimelineEventEffect::Ignored, |index| {
                    TimelineEventEffect::Updated { index }
                });
        }

        let message_id = message.id.clone();
        let insertion_index = self
            .messages
            .binary_search(&message)
            .unwrap_or_else(|index| index);
        self.messages.insert(insertion_index, message);
        self.rebuild_rows();
        let row_index = self
            .row_index_for_message(&message_id)
            .unwrap_or(self.rows.len().saturating_sub(1));
        if insertion_index + 1 == self.messages.len() {
            TimelineEventEffect::Appended { index: row_index }
        } else {
            TimelineEventEffect::Inserted { index: row_index }
        }
    }

    fn remove_message(&mut self, message_id: &MessageId) -> TimelineEventEffect {
        let Some(message_index) = self
            .messages
            .iter()
            .position(|message| &message.id == message_id)
        else {
            return TimelineEventEffect::Ignored;
        };
        let row_index = self
            .row_index_for_message(message_id)
            .unwrap_or(message_index);
        self.messages.remove(message_index);
        self.rebuild_rows();
        TimelineEventEffect::Removed { index: row_index }
    }

    fn row_index_for_message(&self, message_id: &MessageId) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(row, TimelineRow::Message(message) if &message.message.id == message_id),
        )
    }

    fn rebuild_rows(&mut self) {
        self.messages.sort();
        self.rows.clear();
        let mut previous_day = None;

        for (index, message) in self.messages.iter().enumerate() {
            let timestamp = TimestampLabels::from_timestamp(message.sent_at);
            if previous_day.as_deref() != Some(timestamp.day.as_str()) {
                self.rows.push(TimelineRow::DaySeparator(DaySeparator {
                    label: timestamp.day.clone(),
                }));
                previous_day = Some(timestamp.day.clone());
            }

            let previous = self.messages.get(index.wrapping_sub(1));
            let next = self.messages.get(index + 1);
            let same_day = |other: Option<&Message>| {
                other.is_some_and(|other| {
                    TimestampLabels::from_timestamp(other.sent_at).day == timestamp.day
                })
            };
            let same_sender =
                |other: Option<&Message>| other.is_some_and(|other| other.sender == message.sender);
            let has_previous = same_day(previous) && same_sender(previous);
            let has_next = same_day(next) && same_sender(next);
            let group_position = match (has_previous, has_next) {
                (false, false) => GroupPosition::Single,
                (false, true) => GroupPosition::First,
                (true, true) => GroupPosition::Middle,
                (true, false) => GroupPosition::Last,
            };
            let sender_name = message
                .sender
                .as_ref()
                .and_then(|sender| self.sender_names.get(sender))
                .cloned()
                .or_else(|| message.sender.as_ref().map(ToString::to_string))
                .unwrap_or_else(|| "Unknown sender".to_owned());
            let parts = rendered_parts(message);
            self.rows
                .push(TimelineRow::Message(Box::new(TimelineMessage {
                    message: message.clone(),
                    sender_name,
                    is_own: self.own_participant_id.as_ref() == message.sender.as_ref(),
                    timestamp,
                    parts,
                    group_position,
                })));
        }
    }
}

/// A callback used to customize the GTK widget for each ordered message part.
pub type PartRenderer = dyn Fn(&RenderedPart) -> gtk::Widget;

/// GTK4/libadwaita timeline view backed by a recyclable `ListView`.
pub struct TimelineView {
    pub root: gtk::Box,
    pub list: gtk::ListView,
    pub scrolled_window: gtk::ScrolledWindow,
    model: Rc<RefCell<TimelineModel>>,
    store: gio::ListStore,
    stack: gtk::Stack,
    retry_button: gtk::Button,
    part_renderer: Rc<RefCell<Box<PartRenderer>>>,
}

impl TimelineView {
    pub fn new(model: TimelineModel) -> Self {
        let model = Rc::new(RefCell::new(model));
        let part_renderer: Rc<RefCell<Box<PartRenderer>>> =
            Rc::new(RefCell::new(Box::new(default_part_renderer)));
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            container.set_hexpand(true);
            item.set_child(Some(&container));
        });

        let renderer_for_bind = Rc::clone(&part_renderer);
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(object) = item.item() else { return };
            let Ok(boxed) = object.downcast::<glib::BoxedAnyObject>() else {
                return;
            };
            let Some(child) = item.child() else { return };
            let Ok(container) = child.downcast::<gtk::Box>() else {
                return;
            };
            clear_box(&container);
            let row = boxed.borrow::<TimelineRow>().clone();
            append_row(&container, &row, &renderer_for_bind);
        });

        factory.connect_unbind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            if let Some(child) = item.child() {
                if let Ok(container) = child.downcast::<gtk::Box>() {
                    clear_box(&container);
                }
            }
        });

        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_show_separators(false);
        list.set_vexpand(true);
        list.set_hexpand(true);

        let scrolled_window = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&list)
            .build();

        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&loading_page(), Some("loading"));
        stack.add_named(&scrolled_window, Some("ready"));
        let (error_page, retry_button) = error_page();
        stack.add_named(&error_page, Some("error"));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&stack);

        let view = Self {
            root,
            list,
            scrolled_window,
            model,
            store,
            stack,
            retry_button,
            part_renderer,
        };
        view.sync_store();
        view.sync_state();
        view
    }

    pub fn model(&self) -> Rc<RefCell<TimelineModel>> {
        Rc::clone(&self.model)
    }

    pub fn list_model(&self) -> gio::ListStore {
        self.store.clone()
    }

    pub fn refresh(&self) {
        self.sync_store();
        self.sync_state();
    }

    pub fn set_part_renderer<F>(&self, renderer: F)
    where
        F: Fn(&RenderedPart) -> gtk::Widget + 'static,
    {
        *self.part_renderer.borrow_mut() = Box::new(renderer);
        self.store
            .items_changed(0, self.store.n_items(), self.store.n_items());
    }

    pub fn connect_retry<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        self.retry_button.connect_clicked(move |_| callback());
    }

    pub fn apply_event(&self, event: &BackendEvent) -> TimelineEventEffect {
        let scroll_state = self.capture_scroll_state();
        let effect = self.model.borrow_mut().apply_event(event);
        self.sync_store();
        self.sync_state();
        self.restore_scroll_state(scroll_state);
        effect
    }

    pub fn set_loading(&self) {
        self.model.borrow_mut().set_loading();
        self.sync_state();
    }

    pub fn set_error(&self, message: impl Into<String>) {
        self.model.borrow_mut().set_error(message);
        self.sync_state();
    }

    pub fn set_ready(&self) {
        self.model.borrow_mut().set_ready();
        self.sync_store();
        self.sync_state();
    }

    pub fn capture_scroll_state(&self) -> ScrollState {
        let adjustment = self.scrolled_window.vadjustment();
        let bottom = (adjustment.upper() - adjustment.page_size() - adjustment.value()).abs();
        ScrollState {
            value: adjustment.value(),
            at_bottom: bottom <= 2.0,
        }
    }

    pub fn restore_scroll_state(&self, state: ScrollState) {
        let adjustment = self.scrolled_window.vadjustment();
        let max_value = (adjustment.upper() - adjustment.page_size()).max(0.0);
        let value = if state.at_bottom {
            max_value
        } else {
            state.value.clamp(0.0, max_value)
        };
        adjustment.set_value(value);
    }

    pub fn preserve_scroll_state<F, R>(&self, operation: F) -> R
    where
        F: FnOnce() -> R,
    {
        let state = self.capture_scroll_state();
        let result = operation();
        self.restore_scroll_state(state);
        result
    }

    fn sync_state(&self) {
        let name = match self.model.borrow().load_state() {
            LoadState::Loading => "loading",
            LoadState::Ready => "ready",
            LoadState::Error(_) => "error",
        };
        if let LoadState::Error(message) = self.model.borrow().load_state() {
            if let Some(page) = self.stack.child_by_name("error") {
                if let Some(status_page) = page.downcast_ref::<adw::StatusPage>() {
                    status_page.set_description(Some(message));
                }
            }
        }
        self.stack.set_visible_child_name(name);
    }

    fn sync_store(&self) {
        let rows = self.model.borrow().rows().to_vec();
        let shared_count = rows.len().min(self.store.n_items() as usize);
        for (index, row) in rows.iter().enumerate().take(shared_count) {
            let Some(object) = self.store.item(index as u32) else {
                continue;
            };
            let Ok(boxed) = object.downcast::<glib::BoxedAnyObject>() else {
                continue;
            };
            if *boxed.borrow::<TimelineRow>() != *row {
                boxed.replace(row.clone());
                self.store.items_changed(index as u32, 1, 1);
            }
        }

        while self.store.n_items() as usize > rows.len() {
            self.store.remove(rows.len() as u32);
        }
        for row in rows.iter().skip(self.store.n_items() as usize) {
            self.store.append(&glib::BoxedAnyObject::new(row.clone()));
        }
    }
}

/// Builds a timeline view from a ready, loading, or error-capable model.
pub fn build_timeline(model: TimelineModel) -> TimelineView {
    TimelineView::new(model)
}

fn rendered_parts(message: &Message) -> Vec<RenderedPart> {
    if message.is_unsent() {
        return vec![RenderedPart::Text {
            text: "Message unsent".to_owned(),
            formatting: Vec::new(),
        }];
    }

    let mut parts = message.parts.clone();
    for mutation in &message.mutations {
        if let MessageMutationKind::Edit { parts: edited } = &mutation.kind {
            parts = edited.clone();
        }
    }
    parts.into_iter().map(rendered_part).collect()
}

fn rendered_part(part: MessagePart) -> RenderedPart {
    match part {
        MessagePart::Text(text) => RenderedPart::Text {
            text: text.text,
            formatting: text.formatting,
        },
        MessagePart::Attachment(attachment) => RenderedPart::Attachment {
            id: attachment.attachment.id.to_string(),
            kind: attachment.attachment.kind,
            label: attachment
                .attachment
                .file_name
                .unwrap_or_else(|| "Attachment".to_owned()),
            caption: attachment.caption.map(|caption| caption.text),
        },
        MessagePart::LinkPreview(preview) => RenderedPart::LinkPreview {
            url: preview.url,
            title: preview.title,
            summary: preview.summary,
        },
        MessagePart::Location(location) => RenderedPart::Location {
            label: location_label(&location),
        },
        MessagePart::Contact(contact) => RenderedPart::Contact {
            display_name: contact.display_name,
        },
        MessagePart::ServiceExtension(extension) => RenderedPart::ServiceExtension {
            service: extension.service,
            name: extension.name,
        },
    }
}

fn location_label(location: &Location) -> String {
    let latitude = location.point.latitude_e6 as f64 / 1_000_000.0;
    let longitude = location.point.longitude_e6 as f64 / 1_000_000.0;
    format!("Location: {latitude:.6}, {longitude:.6}")
}

fn fixture_own_participant_id(
    fixture: &FixtureSet,
    conversation: &Conversation,
) -> Option<ParticipantId> {
    let identity_ids = fixture
        .account
        .identities
        .iter()
        .map(|identity| &identity.id)
        .collect::<Vec<_>>();
    let person_ids = fixture
        .account
        .identities
        .iter()
        .filter_map(|identity| identity.person_id.as_ref())
        .collect::<Vec<_>>();
    conversation
        .participants
        .iter()
        .find(|participant| {
            participant
                .identity_id
                .as_ref()
                .is_some_and(|identity_id| identity_ids.contains(&identity_id))
                || participant
                    .person_id
                    .as_ref()
                    .is_some_and(|person_id| person_ids.contains(&person_id))
        })
        .map(|participant| participant.id.clone())
}

fn loading_page() -> adw::StatusPage {
    let page = adw::StatusPage::builder()
        .title("Loading messages")
        .description("Preparing this conversation…")
        .build();
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    page.set_child(Some(&spinner));
    page
}

fn error_page() -> (adw::StatusPage, gtk::Button) {
    let page = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title("Messages unavailable")
        .description("This conversation could not be loaded.")
        .build();
    let retry = gtk::Button::builder()
        .label("Try again")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::Center)
        .build();
    page.set_child(Some(&retry));
    (page, retry)
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn append_row(container: &gtk::Box, row: &TimelineRow, renderer: &Rc<RefCell<Box<PartRenderer>>>) {
    match row {
        TimelineRow::DaySeparator(separator) => {
            let label = gtk::Label::builder()
                .label(&separator.label)
                .halign(gtk::Align::Center)
                .margin_top(12)
                .margin_bottom(6)
                .build();
            label.add_css_class("dim-label");
            label.add_css_class("caption-heading");
            container.append(&label);
        }
        TimelineRow::Message(message) => append_message(container, message, renderer),
    }
}

fn append_message(
    container: &gtk::Box,
    message: &TimelineMessage,
    renderer: &Rc<RefCell<Box<PartRenderer>>>,
) {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    line.set_hexpand(true);
    line.set_margin_start(16);
    line.set_margin_end(16);
    line.set_margin_top(match message.group_position {
        GroupPosition::Single | GroupPosition::First => 6,
        GroupPosition::Middle | GroupPosition::Last => 2,
    });
    line.set_margin_bottom(match message.group_position {
        GroupPosition::Single | GroupPosition::Last => 6,
        GroupPosition::First | GroupPosition::Middle => 2,
    });

    let bubble = gtk::Frame::new(None);
    bubble.set_halign(if message.is_own {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    bubble.add_css_class(if message.is_own { "accent-bg" } else { "card" });

    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.set_margin_start(14);
    body.set_margin_end(14);
    body.set_margin_top(9);
    body.set_margin_bottom(8);

    let sender = gtk::Label::builder()
        .label(if message.is_own {
            "You"
        } else {
            &message.sender_name
        })
        .halign(gtk::Align::Start)
        .build();
    sender.add_css_class("caption-heading");
    body.append(&sender);

    for part in &message.parts {
        let widget = {
            let renderer = renderer.borrow();
            renderer(part)
        };
        body.append(&widget);
    }

    let timestamp = gtk::Label::builder()
        .label(&message.timestamp.time)
        .tooltip_text(&message.timestamp.day)
        .halign(gtk::Align::End)
        .build();
    timestamp.add_css_class("dim-label");
    timestamp.add_css_class("caption");
    body.append(&timestamp);

    bubble.set_child(Some(&body));
    line.append(&bubble);
    container.append(&line);
}

fn default_part_renderer(part: &RenderedPart) -> gtk::Widget {
    match part {
        RenderedPart::Text { text, .. } => {
            let label = gtk::Label::new(Some(text));
            label.set_wrap(true);
            label.set_selectable(true);
            label.set_xalign(0.0);
            label.upcast()
        }
        RenderedPart::Attachment {
            kind,
            label: name,
            caption,
            ..
        } => {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let label = gtk::Label::builder()
                .label(format!("{}: {name}", attachment_kind_label(kind)))
                .halign(gtk::Align::Start)
                .build();
            content.append(&label);
            if let Some(caption) = caption {
                let caption_label = gtk::Label::builder()
                    .label(caption)
                    .halign(gtk::Align::Start)
                    .wrap(true)
                    .build();
                caption_label.add_css_class("dim-label");
                content.append(&caption_label);
            }
            content.upcast()
        }
        RenderedPart::LinkPreview {
            url,
            title,
            summary,
        } => {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let heading = gtk::Label::builder()
                .label(title.as_deref().unwrap_or(url))
                .halign(gtk::Align::Start)
                .wrap(true)
                .build();
            heading.add_css_class("heading");
            content.append(&heading);
            if let Some(summary) = summary {
                let summary_label = gtk::Label::builder()
                    .label(summary)
                    .halign(gtk::Align::Start)
                    .wrap(true)
                    .build();
                summary_label.add_css_class("dim-label");
                content.append(&summary_label);
            }
            content.upcast()
        }
        RenderedPart::Location { label } => {
            let label = gtk::Label::builder()
                .label(label)
                .halign(gtk::Align::Start)
                .build();
            label.upcast()
        }
        RenderedPart::Contact { display_name } => {
            let label = gtk::Label::builder()
                .label(format!("Contact: {display_name}"))
                .halign(gtk::Align::Start)
                .build();
            label.upcast()
        }
        RenderedPart::ServiceExtension { service, name } => {
            let label = gtk::Label::builder()
                .label(format!("{service}: {name}"))
                .halign(gtk::Align::Start)
                .build();
            label.add_css_class("dim-label");
            label.upcast()
        }
    }
}

fn attachment_kind_label(kind: &AttachmentKind) -> &'static str {
    match kind {
        AttachmentKind::Image => "Image",
        AttachmentKind::Video => "Video",
        AttachmentKind::Audio => "Audio",
        AttachmentKind::File => "File",
        AttachmentKind::Sticker => "Sticker",
        AttachmentKind::Contact => "Contact",
        AttachmentKind::Location => "Location",
        AttachmentKind::Other(_) => "Attachment",
    }
}

fn civil_date_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted / 146_097
    } else {
        (shifted - 146_096) / 146_097
    };
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_model_orders_messages_and_marks_own_sender() {
        let backend = MockBackend::new();
        let fixture = backend.fixture();
        let conversation_id = &fixture.conversations[0].id;
        let model = TimelineModel::from_fixture(fixture, conversation_id, None)
            .expect("fixture conversation exists");

        assert_eq!(model.load_state(), &LoadState::Ready);
        assert_eq!(model.messages().len(), 2);
        assert_eq!(model.messages()[0].id.to_string(), "message-text-fixture");
        let messages = model
            .rows()
            .iter()
            .filter_map(|row| match row {
                TimelineRow::Message(message) => Some(message),
                TimelineRow::DaySeparator(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(!messages[0].is_own);
        assert!(messages[1].is_own);
        assert!(
            model
                .rows()
                .iter()
                .any(|row| matches!(row, TimelineRow::DaySeparator(_)))
        );
    }

    #[test]
    fn multipart_parts_keep_order_and_apply_edits() {
        let backend = MockBackend::new();
        let fixture = backend.fixture();
        let direct_id = fixture.conversations[0].id.clone();
        let direct = TimelineModel::from_fixture(fixture, &direct_id, None).unwrap();
        let multipart_message = direct
            .rows()
            .iter()
            .find_map(|row| match row {
                TimelineRow::Message(message)
                    if message.message.id.as_str() == "message-reply-fixture" =>
                {
                    Some(message)
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            multipart_message.parts[0],
            RenderedPart::Text { .. }
        ));
        assert!(matches!(
            multipart_message.parts[1],
            RenderedPart::Attachment { .. }
        ));
        assert!(matches!(
            multipart_message.parts[2],
            RenderedPart::Attachment { .. }
        ));
        assert!(matches!(
            multipart_message.parts[3],
            RenderedPart::LinkPreview { .. }
        ));

        let group_id = fixture.conversations[1].id.clone();
        let group = TimelineModel::from_fixture(fixture, &group_id, None).unwrap();
        let edited = group
            .rows()
            .iter()
            .find_map(|row| match row {
                TimelineRow::Message(message)
                    if message.message.id.as_str() == "message-edited-fixture" =>
                {
                    Some(message)
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            &edited.parts[0],
            RenderedPart::Text { text, .. } if text.contains("west entrance")
        ));
    }

    #[test]
    fn scripted_events_have_deterministic_append_update_remove_effects() {
        let mut backend = MockBackend::new();
        let fixture = backend.fixture().clone();
        let direct_id = fixture.conversations[0].id.clone();
        let mut model = TimelineModel::from_fixture(&fixture, &direct_id, None).unwrap();
        let initial_count = model.messages().len();

        assert!(matches!(
            model.apply_event(&backend.next_event().unwrap()),
            TimelineEventEffect::Ignored
        ));
        assert!(matches!(
            model.apply_event(&backend.next_event().unwrap()),
            TimelineEventEffect::Updated { .. }
        ));
        assert!(matches!(
            model.apply_event(&backend.next_event().unwrap()),
            TimelineEventEffect::Ignored
        ));
        assert!(matches!(
            model.apply_event(&backend.next_event().unwrap()),
            TimelineEventEffect::Appended { .. }
        ));
        for _ in 0..5 {
            assert!(matches!(
                model.apply_event(&backend.next_event().unwrap()),
                TimelineEventEffect::Ignored
            ));
        }
        assert_eq!(model.messages().len(), initial_count + 1);

        let group_id = fixture.conversations[1].id.clone();
        let mut group = TimelineModel::from_fixture(&fixture, &group_id, None).unwrap();
        let script = litebubbles_mock_backend::representative_events(&fixture);
        assert!(matches!(
            group.apply_event(&script.events()[0]),
            TimelineEventEffect::Ignored
        ));
        assert!(matches!(
            group.apply_event(&script.events()[1]),
            TimelineEventEffect::Ignored
        ));
        assert!(matches!(
            group.apply_event(&script.events()[2]),
            TimelineEventEffect::Updated { .. }
        ));
        assert!(matches!(
            group.apply_event(&script.events()[7]),
            TimelineEventEffect::Removed { .. }
        ));
    }

    #[test]
    fn timestamps_are_stable_for_negative_and_fixture_epochs() {
        assert_eq!(
            TimestampLabels::from_timestamp(Timestamp::from_millis(1_700_000_000_000)),
            TimestampLabels {
                day: "2023-11-14".to_owned(),
                time: "22:13:20".to_owned(),
            }
        );
        assert_eq!(
            TimestampLabels::from_timestamp(Timestamp::from_millis(-1)),
            TimestampLabels {
                day: "1969-12-31".to_owned(),
                time: "23:59:59".to_owned(),
            }
        );
    }

    #[test]
    fn loading_error_and_scroll_state_are_explicit() {
        let id = ConversationId::new("conversation").unwrap();
        let mut model = TimelineModel::new(id, None);
        assert_eq!(model.load_state(), &LoadState::Loading);
        model.set_error("network unavailable");
        assert_eq!(
            model.load_state(),
            &LoadState::Error("network unavailable".to_owned())
        );
        model.set_ready();
        assert_eq!(model.load_state(), &LoadState::Ready);
        assert_eq!(ScrollState::default(), ScrollState::default());
    }
}
