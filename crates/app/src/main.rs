use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use litebubbles_core::{
    Conversation, ConversationId, ConversationKind, DeliveryState, Message, MessageId, MessagePart,
    Participant, ParticipantId, ParticipantRole, ReadState, TextPart, Timestamp,
};

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

#[derive(Clone, Debug)]
struct DemoConversation {
    conversation: Conversation,
    last_message: Message,
    unread: u32,
}

impl DemoConversation {
    fn title(&self) -> &str {
        self.conversation
            .title
            .as_deref()
            .or_else(|| {
                self.conversation
                    .participants
                    .first()
                    .and_then(|participant| participant.display_name.as_deref())
            })
            .unwrap_or("Conversation")
    }

    fn snippet(&self) -> &str {
        self.last_message
            .parts
            .iter()
            .find_map(|part| match part {
                MessagePart::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .unwrap_or("No message preview")
    }
}

#[derive(Debug)]
struct UiModel {
    state: ViewState,
    selected: Option<usize>,
    conversations: Vec<DemoConversation>,
}

impl UiModel {
    fn demo() -> Self {
        Self {
            state: ViewState::Ready,
            selected: Some(0),
            conversations: vec![
                demo_conversation(
                    "maya-chen",
                    "Maya Chen",
                    "The garden is looking great this morning.",
                    2,
                    18_000,
                ),
                demo_conversation(
                    "weekend-plans",
                    "Weekend plans",
                    "I can bring the picnic blanket.",
                    0,
                    12_000,
                ),
                demo_conversation(
                    "design-circle",
                    "Design circle",
                    "The new color study is ready to review.",
                    1,
                    6_000,
                ),
            ],
        }
    }

    fn select(&mut self, index: usize) {
        if index < self.conversations.len() {
            self.selected = Some(index);
            self.state = ViewState::Ready;
        }
    }

    fn show_empty(&mut self) {
        self.selected = None;
        self.state = ViewState::Empty;
    }

    fn selected_conversation(&self) -> Option<&DemoConversation> {
        self.selected
            .and_then(|index| self.conversations.get(index))
    }
}

fn demo_conversation(
    slug: &str,
    title: &str,
    message_text: &str,
    unread: u32,
    timestamp: i64,
) -> DemoConversation {
    let conversation_id = ConversationId::new(format!("conversation-{slug}")).unwrap();
    let participant_id = ParticipantId::new(format!("participant-{slug}")).unwrap();
    let message_id = MessageId::new(format!("message-{slug}")).unwrap();
    let participant = Participant {
        id: participant_id.clone(),
        person_id: None,
        identity_id: None,
        display_name: Some(title.to_owned()),
        role: ParticipantRole::Member,
        joined_at: Some(Timestamp::from_millis(timestamp - 86_400_000)),
        left_at: None,
        extensions: vec![],
    };
    let conversation = Conversation {
        id: conversation_id.clone(),
        kind: ConversationKind::Direct,
        title: Some(title.to_owned()),
        participants: vec![participant],
        created_at: Some(Timestamp::from_millis(timestamp - 86_400_000)),
        updated_at: Some(Timestamp::from_millis(timestamp)),
        extensions: vec![],
    };
    let last_message = Message {
        id: message_id,
        conversation_id,
        sender: Some(participant_id),
        sent_at: Timestamp::from_millis(timestamp),
        parts: vec![MessagePart::Text(TextPart {
            text: message_text.to_owned(),
            formatting: vec![],
        })],
        mutations: vec![],
        reactions: vec![],
        delivery: DeliveryState::Delivered,
        delivery_receipts: vec![],
        read_state: ReadState::Read {
            at: Timestamp::from_millis(timestamp),
        },
        read_receipts: vec![],
        reply_to: None,
        extensions: vec![],
    };
    DemoConversation {
        conversation,
        last_message,
        unread,
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
    let model = Rc::new(RefCell::new(UiModel::demo()));
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
    for (index, conversation) in model.borrow().conversations.iter().enumerate() {
        let page = conversation_page(conversation, &content_stack);
        content_stack.add_titled(
            &page,
            Some(&format!("conversation-{index}")),
            conversation.title(),
        );
    }
    content_stack.set_visible_child_name("conversation-0");

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
        .hexpand(true)
        .build();
    let search_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);
    search_box.set_margin_bottom(8);
    search_box.append(&search);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    list.add_css_class("navigation-sidebar");
    for conversation in &model.borrow().conversations {
        list.append(&conversation_row(conversation));
    }
    let model_for_search = Rc::clone(model);
    let list_for_search = list.clone();
    search.connect_search_changed(move |entry| {
        let query = entry.text().to_ascii_lowercase();
        let mut row = list_for_search.first_child();
        let mut index = 0;
        while let Some(child) = row {
            let visible = model_for_search
                .borrow()
                .conversations
                .get(index)
                .map(|conversation| {
                    query.is_empty() || conversation.title().to_ascii_lowercase().contains(&query)
                })
                .unwrap_or(false);
            child.set_visible(visible);
            row = child.next_sibling();
            index += 1;
        }
    });

    let model_for_selection = Rc::clone(model);
    let split_for_selection = split_view.clone();
    let stack_for_selection = content_stack.clone();
    list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let index = row.index();
        if index < 0 {
            return;
        }
        let index = index as usize;
        model_for_selection.borrow_mut().select(index);
        stack_for_selection.set_visible_child_name(&format!("conversation-{index}"));
        if split_for_selection.is_collapsed() {
            split_for_selection.set_show_content(true);
        }
    });
    if let Some(row) = list.row_at_index(0) {
        list.select_row(Some(&row));
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

fn conversation_row(conversation: &DemoConversation) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_margin_top(10);
    outer.set_margin_bottom(10);
    let avatar = adw::Avatar::new(40, Some(conversation.title()), true);
    outer.append(&avatar);

    let details = gtk::Box::new(gtk::Orientation::Vertical, 3);
    details.set_hexpand(true);
    let title = gtk::Label::builder()
        .label(conversation.title())
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("heading");
    let snippet = gtk::Label::builder()
        .label(conversation.snippet())
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .lines(1)
        .build();
    snippet.add_css_class("dim-label");
    details.append(&title);
    details.append(&snippet);
    outer.append(&details);
    if conversation.unread > 0 {
        let unread = gtk::Label::new(Some(&conversation.unread.to_string()));
        unread.add_css_class("numeric");
        unread.add_css_class("accent");
        unread.set_valign(gtk::Align::Center);
        outer.append(&unread);
    }
    row.set_child(Some(&outer));
    row
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

fn conversation_page(conversation: &DemoConversation, _stack: &gtk::Stack) -> gtk::ScrolledWindow {
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
        model_for_retry.borrow_mut().state = ViewState::Ready;
        stack_for_retry.set_visible_child_name("conversation-0");
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
    fn demo_rows_are_valid_core_relationships() {
        let model = UiModel::demo();
        assert_eq!(model.conversations.len(), 3);
        for row in &model.conversations {
            assert!(row.conversation.validate().is_ok());
            assert!(row.last_message.validate().is_ok());
            assert_eq!(row.last_message.conversation_id, row.conversation.id);
        }
    }

    #[test]
    fn selection_and_empty_actions_update_shell_state() {
        let mut model = UiModel::demo();
        model.select(2);
        assert_eq!(model.selected, Some(2));
        assert_eq!(model.state, ViewState::Ready);
        model.show_empty();
        assert_eq!(model.selected, None);
        assert_eq!(model.state.page_name(), "empty");
    }

    #[test]
    fn snippets_come_from_ordered_core_message_parts() {
        let model = UiModel::demo();
        assert_eq!(
            model.conversations[0].snippet(),
            "The garden is looking great this morning."
        );
    }
}
