use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use litebubbles_core::{
    Conversation, ConversationId, ConversationKind, Message, MessagePart, PersonId, ReadState,
    Timestamp,
};
use litebubbles_mock_backend::{FixtureSet, MockBackend};

const APPLICATION_ID: &str = "io.github.tannerkrewson.LiteBubbles";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewState {
    Loading,
    Empty,
    Ready,
    Error,
}

impl ViewState {
    const fn page_name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Empty => "empty",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConversationItem {
    id: ConversationId,
    title: String,
    preview: String,
    timestamp: Option<Timestamp>,
    unread: u32,
    participant_fallback: String,
    avatar_available: bool,
    search_text: String,
}

impl ConversationItem {
    fn from_fixture(fixture: &FixtureSet, conversation: &Conversation) -> Self {
        let account_person_ids = fixture
            .account
            .identities
            .iter()
            .filter_map(|identity| identity.person_id.as_ref())
            .collect::<Vec<_>>();
        let participant_names = conversation
            .participants
            .iter()
            .filter_map(|participant| {
                participant_name(
                    fixture,
                    participant.person_id.as_ref(),
                    participant.display_name.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let participant_fallback = participant_names.join(", ");
        let direct_fallback = conversation
            .participants
            .iter()
            .filter(|participant| {
                participant
                    .person_id
                    .as_ref()
                    .is_none_or(|person_id| !account_person_ids.contains(&person_id))
            })
            .find_map(|participant| {
                participant_name(
                    fixture,
                    participant.person_id.as_ref(),
                    participant.display_name.as_deref(),
                )
            });
        let title = conversation
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| match &conversation.kind {
                ConversationKind::Group(details) => details.title.clone(),
                ConversationKind::Direct => direct_fallback,
            })
            .or_else(|| participant_names.first().cloned())
            .unwrap_or_else(|| "Conversation".to_owned());
        let messages = fixture
            .messages_for(&conversation.id)
            .filter(|message| !message.is_unsent())
            .collect::<Vec<_>>();
        let latest_message = messages
            .iter()
            .max_by_key(|message| message.sent_at)
            .copied();
        let unread = messages
            .iter()
            .filter(|message| matches!(&message.read_state, ReadState::Unread))
            .count() as u32;
        let avatar_available = conversation_avatar_available(fixture, conversation);
        let preview = latest_message
            .map(message_preview)
            .unwrap_or_else(|| "No messages yet".to_owned());
        let search_text = format!(
            "{} {} {}",
            title.to_lowercase(),
            participant_fallback.to_lowercase(),
            preview.to_lowercase()
        );

        Self {
            id: conversation.id.clone(),
            title,
            preview,
            timestamp: latest_message
                .map(|message| message.sent_at)
                .or(conversation.updated_at),
            unread,
            participant_fallback,
            avatar_available,
            search_text,
        }
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn snippet(&self) -> &str {
        &self.preview
    }

    fn timestamp(&self) -> Option<Timestamp> {
        self.timestamp
    }

    fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.search_text.contains(&query)
    }
}

fn participant_name(
    fixture: &FixtureSet,
    person_id: Option<&PersonId>,
    participant_name: Option<&str>,
) -> Option<String> {
    participant_name
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            person_id.and_then(|person_id| {
                fixture
                    .people
                    .iter()
                    .find(|person| &person.id == person_id)
                    .map(|person| person.display_name.clone())
            })
        })
}

fn conversation_avatar_available(fixture: &FixtureSet, conversation: &Conversation) -> bool {
    match &conversation.kind {
        ConversationKind::Group(details) => details.avatar.is_some(),
        ConversationKind::Direct => conversation.participants.iter().any(|participant| {
            participant.person_id.as_ref().is_some_and(|person_id| {
                fixture
                    .people
                    .iter()
                    .find(|person| &person.id == person_id)
                    .and_then(|person| person.avatar.as_ref())
                    .is_some()
            })
        }),
    }
}

fn message_preview(message: &Message) -> String {
    message
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePart::Text(text) => Some(text.text.clone()),
            MessagePart::Attachment(attachment) => attachment
                .caption
                .as_ref()
                .map(|caption| caption.text.clone())
                .or_else(|| Some("Attachment".to_owned())),
            MessagePart::LinkPreview(link) => {
                link.title.clone().or_else(|| Some("Link".to_owned()))
            }
            MessagePart::Location(_) => Some("Location".to_owned()),
            MessagePart::Contact(contact) => Some(contact.display_name.clone()),
            MessagePart::ServiceExtension(_) => None,
        })
        .unwrap_or_else(|| "No message preview".to_owned())
}

fn timestamp_text(timestamp: Option<Timestamp>) -> String {
    timestamp
        .and_then(|timestamp| {
            glib::DateTime::from_unix_local(timestamp.as_millis() / 1_000)
                .ok()
                .and_then(|date| date.format("%b %-d").ok())
                .map(|text| text.to_string())
        })
        .unwrap_or_default()
}

#[derive(Debug)]
struct UiModel {
    state: ViewState,
    selected: Option<ConversationId>,
    conversations: Vec<ConversationItem>,
}

impl UiModel {
    fn fixture() -> Self {
        let backend = MockBackend::new();
        Self::from_fixture(backend.fixture())
    }

    fn from_fixture(fixture: &FixtureSet) -> Self {
        let conversations = fixture
            .conversations
            .iter()
            .map(|conversation| ConversationItem::from_fixture(fixture, conversation))
            .collect::<Vec<_>>();
        let selected = conversations.first().map(|item| item.id.clone());
        Self {
            state: ViewState::Ready,
            selected,
            conversations,
        }
    }

    fn select(&mut self, id: &ConversationId) {
        if self
            .conversations
            .iter()
            .any(|conversation| conversation.id == *id)
        {
            self.selected = Some(id.clone());
            self.state = ViewState::Ready;
        }
    }

    fn show_empty(&mut self) {
        self.selected = None;
        self.state = ViewState::Empty;
    }

    fn selected_conversation(&self) -> Option<&ConversationItem> {
        self.selected
            .as_ref()
            .and_then(|id| self.conversations.iter().find(|item| &item.id == id))
    }
}

fn main() -> glib::ExitCode {
    adw::init().expect("libadwaita must initialize");

    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    application.connect_activate(build_window);
    application.run()
}

fn build_window(application: &adw::Application) {
    let model = Rc::new(RefCell::new(UiModel::fixture()));
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("LiteBubbles")
        .default_width(1_080)
        .default_height(720)
        .build();

    let content_stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    append_state_pages(&content_stack);
    for conversation in &model.borrow().conversations {
        let page = conversation_page(conversation, &content_stack);
        content_stack.add_titled(
            &page,
            Some(&conversation_page_name(&conversation.id)),
            conversation.title(),
        );
    }
    if let Some(selected) = model.borrow().selected_conversation() {
        content_stack.set_visible_child_name(&conversation_page_name(&selected.id));
    } else {
        content_stack.set_visible_child_name(ViewState::Empty.page_name());
    }

    let split_view = adw::NavigationSplitView::builder()
        .min_sidebar_width(260.0)
        .max_sidebar_width(380.0)
        .sidebar_width_fraction(0.34)
        .build();
    let (sidebar, search_entry) = conversation_sidebar(&model, &split_view, &content_stack);
    let sidebar_page = adw::NavigationPage::new(&sidebar, "Conversations");
    let content_toolbar = content_toolbar(&content_stack, &model);
    let content_page = adw::NavigationPage::new(&content_toolbar, "Messages");
    split_view.set_sidebar(Some(&sidebar_page));
    split_view.set_content(Some(&content_page));

    let narrow_breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 700sp")
            .expect("the narrow-window breakpoint is valid"),
    );
    narrow_breakpoint.add_setter(&split_view, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(narrow_breakpoint);
    window.set_content(Some(&split_view));

    install_window_actions(&window, &model, &split_view, &content_stack, &search_entry);
    install_application_actions(application, &window, &model, &split_view, &content_stack);
    window.present();
}

fn append_state_pages(stack: &gtk::Stack) {
    let loading = adw::StatusPage::builder()
        .title("Connecting to LiteBubbles")
        .description("Preparing your conversations…")
        .build();
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    loading.set_child(Some(&spinner));
    stack.add_named(&loading, Some(ViewState::Loading.page_name()));

    let empty = adw::StatusPage::builder()
        .icon_name("mail-send-symbolic")
        .title("No conversation selected")
        .description("Choose a conversation or start a new one to begin.")
        .build();
    stack.add_named(&empty, Some(ViewState::Empty.page_name()));

    let error = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title("Conversations are unavailable")
        .description("LiteBubbles could not load the conversation list.")
        .build();
    let retry = gtk::Button::builder()
        .label("Try again")
        .css_classes(["suggested-action"])
        .action_name("win.retry")
        .halign(gtk::Align::Center)
        .build();
    error.set_child(Some(&retry));
    stack.add_named(&error, Some(ViewState::Error.page_name()));
}

fn conversation_sidebar(
    model: &Rc<RefCell<UiModel>>,
    split_view: &adw::NavigationSplitView,
    content_stack: &gtk::Stack,
) -> (adw::ToolbarView, gtk::SearchEntry) {
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("Messages", "LiteBubbles");
    header.set_title_widget(Some(&title));
    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New conversation (Ctrl+N)")
        .action_name("app.new-conversation")
        .build();
    header.pack_end(&new_button);
    let menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Application menu")
        .menu_model(&application_menu())
        .build();
    header.pack_end(&menu);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search conversations")
        .tooltip_text("Search conversations (Ctrl+F)")
        .hexpand(true)
        .build();
    search.set_accessible_role(gtk::AccessibleRole::SearchBox);
    let search_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);
    search_box.set_margin_bottom(8);
    search_box.append(&search);

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for conversation in &model.borrow().conversations {
        store.append(&glib::BoxedAnyObject::new(conversation.clone()));
    }

    let query = Rc::new(RefCell::new(String::new()));
    let query_for_filter = Rc::clone(&query);
    let filter = gtk::CustomFilter::new(move |object| {
        object
            .downcast_ref::<glib::BoxedAnyObject>()
            .and_then(|item| item.try_borrow::<ConversationItem>().ok())
            .is_some_and(|item| item.matches_query(&query_for_filter.borrow()))
    });
    let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    filtered.set_incremental(true);
    let selection = gtk::SingleSelection::new(Some(filtered.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let list_item = object
            .downcast_ref::<gtk::ListItem>()
            .expect("list item factory setup receives GtkListItem");
        list_item.set_selectable(true);
        list_item.set_activatable(true);
        list_item.set_focusable(true);
        list_item.connect_selected_notify(update_row_selection);
        list_item.set_child(Some(&conversation_row_widget()));
    });
    factory.connect_bind(|_, object| {
        let list_item = object
            .downcast_ref::<gtk::ListItem>()
            .expect("list item factory bind receives GtkListItem");
        bind_conversation_row(list_item);
    });
    factory.connect_unbind(|_, object| {
        let list_item = object
            .downcast_ref::<gtk::ListItem>()
            .expect("list item factory unbind receives GtkListItem");
        list_item.set_accessible_label("");
        list_item.set_accessible_description("");
    });

    let list = gtk::ListView::new(Some(selection.clone()), Some(factory.clone()));
    list.set_vexpand(true);
    list.set_single_click_activate(true);
    list.set_show_separators(false);
    list.add_css_class("navigation-sidebar");
    list.set_accessible_role(gtk::AccessibleRole::ListBox);

    let model_for_search = Rc::clone(model);
    let query_for_search = Rc::clone(&query);
    let filter_for_search = filter.clone();
    let selection_for_search = selection.clone();
    let filtered_for_search = filtered.clone();
    search.connect_search_changed(move |entry| {
        query_for_search.replace(entry.text().trim().to_lowercase());
        filter_for_search.changed(gtk::FilterChange::Different);

        let selected_id = model_for_search.borrow().selected.clone();
        let selected_position = selected_id.and_then(|id| {
            (0..filtered_for_search.n_items()).find(|position| {
                filtered_for_search
                    .item(*position)
                    .map(|object| {
                        object
                            .downcast::<glib::BoxedAnyObject>()
                            .ok()
                            .and_then(|item| {
                                item.try_borrow::<ConversationItem>()
                                    .ok()
                                    .map(|item| item.id == id)
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
        });
        if let Some(position) = selected_position {
            selection_for_search.set_selected(position);
        } else if filtered_for_search.n_items() > 0 {
            selection_for_search.set_selected(0);
        } else {
            selection_for_search.set_selected(gtk::INVALID_LIST_POSITION);
        }
    });

    let search_for_stop = search.clone();
    search.connect_stop_search(move |_| {
        search_for_stop.set_text("");
    });

    let model_for_selection = Rc::clone(model);
    let split_for_selection = split_view.clone();
    let stack_for_selection = content_stack.clone();
    selection.connect_selected_item_notify(move |selection| {
        let Some(object) = selection.selected_item() else {
            return;
        };
        let Ok(item) = object.downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let Ok(item) = item.try_borrow::<ConversationItem>() else {
            return;
        };
        let id = item.id.clone();
        drop(item);
        model_for_selection.borrow_mut().select(&id);
        stack_for_selection.set_visible_child_name(&conversation_page_name(&id));
        if split_for_selection.is_collapsed() {
            split_for_selection.set_show_content(true);
        }
    });
    if filtered.n_items() > 0 {
        selection.set_selected(0);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&search_box);
    content.append(&scroll);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    (toolbar, search)
}

fn conversation_page_name(id: &ConversationId) -> String {
    format!("conversation-{}", id.as_str())
}

fn conversation_row_widget() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.set_hexpand(true);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_margin_top(10);
    outer.set_margin_bottom(10);
    outer.add_css_class("conversation-row");
    outer.set_accessible_role(gtk::AccessibleRole::ListItem);

    let avatar = adw::Avatar::new(40, None, true);
    avatar.set_valign(gtk::Align::Start);
    outer.append(&avatar);

    let details = gtk::Box::new(gtk::Orientation::Vertical, 3);
    details.set_hexpand(true);
    details.set_valign(gtk::Align::Center);
    let title = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("heading");
    let preview = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .lines(1)
        .build();
    preview.add_css_class("dim-label");
    details.append(&title);
    details.append(&preview);

    let metadata = gtk::Box::new(gtk::Orientation::Vertical, 4);
    metadata.set_valign(gtk::Align::Start);
    let timestamp = gtk::Label::new(None);
    timestamp.add_css_class("caption");
    timestamp.add_css_class("dim-label");
    let unread = gtk::Label::new(None);
    unread.add_css_class("numeric");
    unread.add_css_class("accent");
    unread.set_visible(false);
    metadata.append(&timestamp);
    metadata.append(&unread);

    outer.append(&details);
    outer.append(&metadata);
    outer
}

fn bind_conversation_row(list_item: &gtk::ListItem) {
    let Some(object) = list_item.item() else {
        return;
    };
    let Ok(item) = object.downcast::<glib::BoxedAnyObject>() else {
        return;
    };
    let Ok(item) = item.try_borrow::<ConversationItem>() else {
        return;
    };
    let Some(root) = list_item
        .child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    let Some(avatar) = root
        .first_child()
        .and_then(|child| child.downcast::<adw::Avatar>().ok())
    else {
        return;
    };
    let Some(details) = avatar
        .next_sibling()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    let Some(title) = details
        .first_child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    let Some(preview) = title
        .next_sibling()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    let Some(metadata) = details
        .next_sibling()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    let Some(timestamp) = metadata
        .first_child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    let Some(unread) = timestamp
        .next_sibling()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
    else {
        return;
    };

    let title_text = item.title().to_owned();
    let preview_text = item.snippet();
    let timestamp_text = timestamp_text(item.timestamp());
    let unread_text = item.unread.to_string();
    let accessible_description = if item.unread == 0 {
        format!(
            "{}, {}, {}",
            item.participant_fallback, preview_text, timestamp_text
        )
    } else {
        format!(
            "{}, {}, {}, {} unread",
            item.participant_fallback, preview_text, timestamp_text, item.unread
        )
    };
    avatar.set_text(Some(&title_text));
    avatar.set_custom_image(None::<&gtk::gdk::Paintable>);
    avatar.set_icon_name(None);
    avatar.set_tooltip_text(if item.avatar_available {
        Some("Conversation avatar")
    } else {
        None
    });
    title.set_label(&title_text);
    preview.set_label(preview_text);
    timestamp.set_label(&timestamp_text);
    unread.set_label(&unread_text);
    unread.set_visible(item.unread > 0);
    list_item.set_accessible_label(&title_text);
    list_item.set_accessible_description(&accessible_description);
    root.set_tooltip_text(Some(&format!(
        "{} — {}",
        item.participant_fallback, preview_text
    )));
    update_row_selection(list_item);
}

fn update_row_selection(list_item: &gtk::ListItem) {
    let Some(root) = list_item
        .child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    root.set_css_classes(if list_item.is_selected() {
        &["conversation-row", "selected"]
    } else {
        &["conversation-row"]
    });
}

fn content_toolbar(content_stack: &gtk::Stack, model: &Rc<RefCell<UiModel>>) -> adw::ToolbarView {
    let header = adw::HeaderBar::new();
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Show conversations")
        .action_name("win.show-sidebar")
        .build();
    header.pack_start(&back);
    let selected_title = model
        .borrow()
        .selected_conversation()
        .map(|conversation| conversation.title().to_owned())
        .unwrap_or_else(|| "Messages".to_owned());
    let title = adw::WindowTitle::new(&selected_title, "Conversation");
    header.set_title_widget(Some(&title));
    let call = gtk::Button::builder()
        .icon_name("call-start-symbolic")
        .tooltip_text("Start an audio call")
        .sensitive(false)
        .build();
    header.pack_end(&call);
    let info = gtk::Button::builder()
        .icon_name("info-outline-symbolic")
        .tooltip_text("Conversation details")
        .sensitive(false)
        .build();
    header.pack_end(&info);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content_stack));
    toolbar
}

fn conversation_page(conversation: &ConversationItem, _stack: &gtk::Stack) -> gtk::ScrolledWindow {
    let messages = gtk::Box::new(gtk::Orientation::Vertical, 12);
    messages.set_margin_start(24);
    messages.set_margin_end(24);
    messages.set_margin_top(24);
    messages.set_margin_bottom(12);
    messages.set_valign(gtk::Align::End);

    let incoming = message_bubble(conversation.title(), conversation.snippet(), false);
    messages.append(&incoming);
    let reply = message_bubble(
        "You",
        "Thanks for the update — I’ll take a look this afternoon.",
        true,
    );
    messages.append(&reply);

    let composer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    composer.set_margin_start(16);
    composer.set_margin_end(16);
    composer.set_margin_top(8);
    composer.set_margin_bottom(16);
    let entry = gtk::Entry::builder()
        .placeholder_text("Write a message")
        .hexpand(true)
        .build();
    let send = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text("Send message")
        .sensitive(false)
        .build();
    composer.append(&entry);
    composer.append(&send);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_vexpand(true);
    body.append(&messages);
    body.append(&composer);
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&body)
        .build()
}

fn message_bubble(sender: &str, text: &str, outgoing: bool) -> gtk::Frame {
    let frame = gtk::Frame::new(None);
    frame.set_halign(if outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    frame.add_css_class(if outgoing { "accent-bg" } else { "card" });
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.set_margin_start(14);
    body.set_margin_end(14);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    let sender_label = gtk::Label::builder()
        .label(sender)
        .halign(gtk::Align::Start)
        .build();
    sender_label.add_css_class("caption-heading");
    let text_label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .selectable(true)
        .halign(gtk::Align::Start)
        .build();
    body.append(&sender_label);
    body.append(&text_label);
    frame.set_child(Some(&body));
    frame
}

fn application_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("New conversation"), Some("app.new-conversation"));
    menu.append(Some("Search conversations"), Some("win.search"));
    menu.append(Some("Keyboard shortcuts"), Some("app.shortcuts"));
    menu.append(Some("About LiteBubbles"), Some("app.about"));
    menu.append(Some("Quit"), Some("app.quit"));
    menu
}

fn install_window_actions(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<UiModel>>,
    split_view: &adw::NavigationSplitView,
    content_stack: &gtk::Stack,
    search_entry: &gtk::SearchEntry,
) {
    let search = gio::SimpleAction::new("search", None);
    let search_target = search_entry.clone();
    let split_for_search = split_view.clone();
    search.connect_activate(move |_, _| {
        if split_for_search.is_collapsed() {
            split_for_search.set_show_content(false);
        }
        let _ = search_target.grab_focus();
    });
    window.add_action(&search);

    let show_sidebar = gio::SimpleAction::new("show-sidebar", None);
    let split_for_sidebar = split_view.clone();
    show_sidebar.connect_activate(move |_, _| {
        if split_for_sidebar.is_collapsed() {
            split_for_sidebar.set_show_content(false);
        }
    });
    window.add_action(&show_sidebar);

    let retry = gio::SimpleAction::new("retry", None);
    let model_for_retry = Rc::clone(model);
    let stack_for_retry = content_stack.clone();
    retry.connect_activate(move |_, _| {
        let mut model = model_for_retry.borrow_mut();
        model.state = ViewState::Ready;
        if let Some(first) = model.conversations.first() {
            let first_id = first.id.clone();
            model.selected = Some(first_id.clone());
            stack_for_retry.set_visible_child_name(&conversation_page_name(&first_id));
        }
    });
    window.add_action(&retry);
}

fn install_application_actions(
    application: &adw::Application,
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<UiModel>>,
    split_view: &adw::NavigationSplitView,
    content_stack: &gtk::Stack,
) {
    let new_conversation = gio::SimpleAction::new("new-conversation", None);
    let model_for_new = Rc::clone(model);
    let split_for_new = split_view.clone();
    let stack_for_new = content_stack.clone();
    new_conversation.connect_activate(move |_, _| {
        model_for_new.borrow_mut().show_empty();
        stack_for_new.set_visible_child_name(ViewState::Empty.page_name());
        if split_for_new.is_collapsed() {
            split_for_new.set_show_content(true);
        }
    });
    application.add_action(&new_conversation);

    let shortcuts = gio::SimpleAction::new("shortcuts", None);
    let shortcuts_parent = window.clone();
    shortcuts.connect_activate(move |_, _| {
        let dialog = adw::AlertDialog::new(
            Some("Keyboard shortcuts"),
            Some("Ctrl+N  New conversation\nCtrl+F  Search conversations\nCtrl+Q  Quit"),
        );
        dialog.add_response("close", "Close");
        dialog.set_close_response("close");
        dialog.present(Some(&shortcuts_parent));
    });
    application.add_action(&shortcuts);

    let about = gio::SimpleAction::new("about", None);
    let about_parent = window.clone();
    about.connect_activate(move |_, _| {
        let dialog = adw::AboutDialog::new();
        dialog.set_application_name("LiteBubbles");
        dialog.set_application_icon("io.github.tannerkrewson.LiteBubbles");
        dialog.set_version("0.1.0");
        dialog.set_comments("A native, backend-independent messaging shell for GNOME.");
        dialog.set_developer_name("LiteBubbles contributors");
        dialog.present(Some(&about_parent));
    });
    application.add_action(&about);

    let quit = gio::SimpleAction::new("quit", None);
    let application_for_quit = application.clone();
    quit.connect_activate(move |_, _| application_for_quit.quit());
    application.add_action(&quit);

    application.set_accels_for_action("app.new-conversation", &["<Primary>n"]);
    application.set_accels_for_action("win.search", &["<Primary>f"]);
    application.set_accels_for_action("app.quit", &["<Primary>q"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_rows_are_valid_core_relationships() {
        let backend = MockBackend::new();
        let fixture = backend.fixture();
        let model = UiModel::fixture();
        assert_eq!(model.conversations.len(), 2);
        for conversation in &fixture.conversations {
            assert!(conversation.validate().is_ok());
            let latest = fixture
                .messages_for(&conversation.id)
                .max_by_key(|message| message.sent_at)
                .expect("representative fixture rows have messages");
            assert!(latest.validate().is_ok());
            assert!(
                model
                    .conversations
                    .iter()
                    .any(|row| row.id == conversation.id)
            );
        }
    }

    #[test]
    fn selection_and_empty_actions_update_shell_state() {
        let mut model = UiModel::fixture();
        let selected = model.conversations[1].id.clone();
        model.select(&selected);
        assert_eq!(model.selected, Some(selected));
        assert_eq!(model.state, ViewState::Ready);
        model.show_empty();
        assert_eq!(model.selected, None);
        assert_eq!(model.state.page_name(), "empty");
    }

    #[test]
    fn fixture_rows_expose_fallbacks_previews_timestamps_and_unread_counts() {
        let model = UiModel::fixture();
        let direct = &model.conversations[0];
        assert_eq!(direct.title(), "Maya Fixture");
        assert_eq!(direct.participant_fallback, "Alex Fixture, Maya Fixture");
        assert_eq!(direct.snippet(), "I took a look—here is the latest.");
        assert_eq!(direct.unread, 1);
        assert!(direct.avatar_available);
        assert!(!timestamp_text(direct.timestamp()).is_empty());
    }

    #[test]
    fn search_matches_titles_participants_and_latest_previews() {
        let model = UiModel::fixture();
        assert!(model.conversations[0].matches_query("maya"));
        assert!(model.conversations[1].matches_query("failed delivery"));
        assert!(!model.conversations[0].matches_query("weekend"));
    }

    #[test]
    fn unread_count_ignores_removed_fixture_messages() {
        let model = UiModel::fixture();
        assert_eq!(model.conversations[1].unread, 2);
        assert_eq!(
            model.conversations[1].snippet(),
            "This message demonstrates a failed delivery."
        );
    }
}
