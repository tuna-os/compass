//! Create Extension in the launcher: the form, then the success page.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the form and the page's text are in
//! [`crate::developer_page`].

use iced::keyboard::{Key, key::Named};

use super::{
    Element, LauncherApp, Length, Message, Padding, Page, Task, column, container, scrollable,
};
use crate::developer_page::{self, CreatedPage};
use crate::preferences_page::Purpose;

impl LauncherApp {
    /// Opens the Create Extension form.
    pub(super) fn open_create_extension(&mut self) -> Task<Message> {
        self.page = Page::Preferences(Box::new(developer_page::form()));
        iced::widget::operation::focus_next()
    }

    /// Submits the Create Extension form. `None` when the form showing is
    /// not that one.
    pub(super) fn submit_create_extension(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        if page.purpose != Purpose::CreateExtension {
            return None;
        }
        if let Err(missing) = page.submission() {
            page.notice = Some(format!("Form has errors: fill in {}", missing.join(", ")));
            return Some(Task::none());
        }
        let draft = developer_page::draft(page);
        let Some(backend) = self.backend.clone() else {
            page.notice = Some("Create Extension needs the Compass engine".to_owned());
            return Some(Task::none());
        };
        let title = draft.title.clone();
        Some(Task::perform(
            async move { backend.create_extension(draft).await },
            move |result| Message::ExtensionCreated {
                title: title.clone(),
                result,
            },
        ))
    }

    /// Handles the Create Extension messages.
    pub(super) fn developer_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // `popSelf()` then the success view: the form is replaced.
            Message::ExtensionCreated {
                title,
                result: Ok(path),
            } => {
                self.page = Page::Created(CreatedPage::new(&title, path));
                Task::none()
            }
            Message::ExtensionCreated {
                result: Err(reason),
                ..
            } => {
                if let Page::Preferences(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::CreatedFolderOpened(Ok(())) => self.conceal(),
            Message::CreatedFolderOpened(Err(reason)) => {
                if let Page::Created(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The success page's keys: Enter opens the new extension's folder, as
    /// the C++'s "Open in …" actions do; Escape goes back.
    pub(super) fn created_page_key(&mut self, key: &Key) -> Task<Message> {
        let Page::Created(page) = &self.page else {
            return Task::none();
        };
        match key.as_ref() {
            Key::Named(Named::Escape) => self.update(Message::Back),
            Key::Named(Named::Enter) => {
                let (Some(backend), path) = (self.backend.clone(), page.path.clone()) else {
                    return Task::none();
                };
                Task::perform(
                    async move { backend.open_file(path, false).await },
                    Message::CreatedFolderOpened,
                )
            }
            _ => Task::none(),
        }
    }

    /// The success page's body.
    pub(super) fn created_body<'a>(&'a self, page: &'a CreatedPage) -> Element<'a, Message> {
        let markdown = iced::widget::markdown::view(&page.markdown, self.markdown_settings())
            .map(Message::ExtensionLinkClicked);
        let mut body = column![
            markdown,
            iced::widget::text("Enter: open the folder    Esc: back")
                .font(self.font())
                .size(12)
        ]
        .spacing(10);
        if let Some(notice) = &page.notice {
            body = body.push(self.notice(notice));
        }
        scrollable(container(body).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }
}
