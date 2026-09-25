//! Calculator History in the launcher, and remembering a copied answer.
//!
//! Copying the calculator's answer — from the root list or from this view's
//! live result — remembers it, as the C++ `CopyCalculatorAnswerAction`
//! does; the history's rows offer the C++ panel: pin or unpin, the three
//! copies with the answer primary, then remove and remove all.

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
    text,
};
use crate::action_panel::Action;
use crate::backend::CalculatorChange;
use crate::calculator_page::{self, CalcRow, CalculatorPage};

const PIN: &str = "calc.pin";
const UNPIN: &str = "calc.unpin";
const COPY_ANSWER: &str = "calc.copy-answer";
const COPY_QUESTION: &str = "calc.copy-question";
const COPY_EXPRESSION: &str = "calc.copy-expression";
const REMOVE: &str = "calc.remove";
const REMOVE_ALL: &str = "calc.remove-all";

const NEEDS_ENGINE: &str =
    "Calculator History needs the Compass engine, and this window is running without one";

/// The calculator's own HUD, as `CopyCalculatorAnswerAction`.
#[must_use]
pub fn answer_copied() -> crate::hud::Hud {
    crate::hud::Hud::new("Answer copied to clipboard").with_icon(crate::hud::COPY_ICON)
}

impl LauncherApp {
    /// Opens Calculator History and asks for the rows.
    pub(super) fn open_calculator_history(&mut self) -> Task<Message> {
        self.page = Page::Calculator(CalculatorPage::default());
        Task::batch([self.calculator_query_task(), focus_search()])
    }

    fn calculator_query_task(&mut self) -> Task<Message> {
        let Page::Calculator(page) = &mut self.page else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            let generation = page.generation;
            page.apply(generation, Err(NEEDS_ENGINE.to_owned()));
            return Task::none();
        };
        let (query, generation) = (page.query.clone(), page.generation);
        Task::perform(
            async move { backend.calculator_history(query).await },
            move |result| Message::CalculatorLoaded { generation, result },
        )
    }

    /// Copies `answer`, remembering the calculation in the history.
    pub(super) fn copy_calculation(
        &self,
        question: String,
        answer: String,
        copied: String,
    ) -> Task<Message> {
        let copy = iced::clipboard::write(copied);
        let Some(backend) = self.backend.clone() else {
            return copy;
        };
        let conversion = compass_core::calculator::is_conversion(&question);
        let remember = Task::future(async move {
            if let Err(error) = backend
                .add_calculator_record(question, answer, conversion)
                .await
            {
                tracing::warn!(%error, "could not remember a calculation");
            }
        })
        .discard();
        Task::batch([copy, remember])
    }

    fn edit_calculator(&mut self, change: CalculatorChange, said: &'static str) -> Task<Message> {
        self.panel = None;
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::batch([
            Task::perform(
                async move { backend.edit_calculator_history(change).await },
                move |result| Message::CalculatorEdited(result.map(|()| said)),
            ),
            focus_search(),
        ])
    }

    /// Runs the selected row's primary action: copy the answer.
    fn copy_selected_calculation(&mut self) -> Task<Message> {
        let Page::Calculator(page) = &self.page else {
            return Task::none();
        };
        match page.selected_row() {
            Some(CalcRow::Live(answer)) => {
                let (question, answer) = (answer.question.clone(), answer.answer.clone());
                let copy = self.copy_calculation(question, answer.clone(), answer);
                Task::batch([copy, self.show_hud(answer_copied())])
            }
            Some(CalcRow::Record(record)) => {
                let answer = record.answer.clone();
                self.copy_with_hud(answer)
            }
            None => Task::none(),
        }
    }

    /// The panel over the selected row.
    pub(super) fn open_calculator_panel(&mut self) -> Option<Task<Message>> {
        let Page::Calculator(page) = &self.page else {
            return None;
        };
        let sections = match page.selected_row()? {
            CalcRow::Live(_) => vec![PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new("Copy Result")
                        .with_id(COPY_ANSWER)
                        .with_shortcut("enter"),
                    Action::new("Copy Question And Answer").with_id(COPY_EXPRESSION),
                ],
            }],
            CalcRow::Record(record) => vec![
                PanelSection {
                    name: String::new(),
                    actions: vec![if record.pinned {
                        Action::new("Unpin entry").with_id(UNPIN)
                    } else {
                        Action::new("Pin entry").with_id(PIN)
                    }],
                },
                PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Copy answer")
                            .with_id(COPY_ANSWER)
                            .with_shortcut("enter"),
                        Action::new("Copy question").with_id(COPY_QUESTION),
                        Action::new("Copy question and answer").with_id(COPY_EXPRESSION),
                    ],
                },
                PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Delete entry").with_id(REMOVE),
                        Action::new("Delete all entries").with_id(REMOVE_ALL),
                    ],
                },
            ],
        };
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Calculator History panel action, if `id` is one.
    pub(super) fn calculator_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Calculator(page) = &self.page else {
            return None;
        };
        if !id.starts_with("calc.") {
            return None;
        }
        let row = page.selected_row()?;
        let (question, answer, record_id) = match row {
            CalcRow::Live(answer) => (answer.question.clone(), answer.answer.clone(), None),
            CalcRow::Record(record) => (
                record.question.clone(),
                record.answer.clone(),
                Some(record.id.clone()),
            ),
        };
        let expression = compass_core::calculator_history::live_result_title(&question, &answer);
        self.panel = None;
        Some(match (id, record_id) {
            (COPY_ANSWER, _) => return Some(self.copy_selected_calculation()),
            (COPY_EXPRESSION, None) => {
                let copy = self.copy_calculation(question, answer, expression);
                Task::batch([copy, self.show_hud(answer_copied())])
            }
            (COPY_EXPRESSION, Some(_)) => self.copy_with_hud(expression),
            (COPY_QUESTION, Some(_)) => self.copy_with_hud(question),
            (PIN, Some(id)) => self.edit_calculator(CalculatorChange::Pin(id), "Entry pinned"),
            (UNPIN, Some(id)) => {
                self.edit_calculator(CalculatorChange::Unpin(id), "Entry unpinned")
            }
            (REMOVE, Some(id)) => {
                self.edit_calculator(CalculatorChange::Remove(id), "Entry removed")
            }
            (REMOVE_ALL, Some(_)) => {
                self.edit_calculator(CalculatorChange::RemoveAll, "All entries removed")
            }
            _ => Task::none(),
        })
    }

    /// The view's keys.
    pub(super) fn calculator_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Calculator(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.copy_selected_calculation(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected =
                next_selection(page.len(), page.selected, direction, self.wrap_navigation);
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the view's messages.
    pub(super) fn calculator_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CalculatorQueryChanged(query) => {
                if let Page::Calculator(page) = &mut self.page {
                    page.notice = None;
                    page.set_query(query);
                }
                self.calculator_query_task()
            }
            Message::CalculatorLoaded { generation, result } => {
                if let Page::Calculator(page) = &mut self.page {
                    page.apply(generation, result);
                }
                crate::scroll::reveal_root_selection()
            }
            Message::CalculatorSelected(position) => {
                if let Page::Calculator(page) = &mut self.page
                    && position < page.len()
                {
                    page.selected = position;
                }
                self.copy_selected_calculation()
            }
            Message::CalculatorEdited(result) => {
                if let Page::Calculator(page) = &mut self.page {
                    page.notice = Some(match result {
                        Ok(said) => said.to_owned(),
                        Err(reason) => reason,
                    });
                    page.generation += 1;
                }
                self.calculator_query_task()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn calculator_body<'a>(&'a self, page: &'a CalculatorPage) -> Element<'a, Message> {
        if page.is_empty() {
            let empty = match (&page.failure, page.loaded) {
                (Some(reason), _) => reason.as_str(),
                (None, false) => "Reading the calculator history…",
                (None, true) if page.query.is_empty() => calculator_page::EMPTY,
                (None, true) => "No calculations match",
            };
            return self.notice(empty);
        }
        let heading = |name: String| -> Element<'a, Message> {
            container(text(name).font(self.font()).size(12))
                .padding(Padding::new(4.0).left(10))
                .into()
        };
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        let mut position = 0;
        let push_row = |list: iced::widget::Column<'a, Message>,
                        badge: &str,
                        title: String,
                        subtitle: String,
                        position: usize| {
            let selected = position == page.selected;
            let item = self.list_row(
                self.initial_badge(badge, selected),
                title,
                self.subtitles.then_some(subtitle),
                selected,
            );
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::CalculatorSelected(position))
                .into();
            let item: Element<Message> = if selected {
                container(item).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                item
            };
            list.push(item)
        };
        if let Some(live) = &page.live {
            list = list.push(heading("Calculator".to_owned()));
            list = push_row(
                list,
                "=",
                compass_core::calculator_history::live_result_title(&live.question, &live.answer),
                "Copy to keep it".to_owned(),
                position,
            );
            position += 1;
        }
        for group in &page.groups {
            list = list.push(heading(group.name.clone()));
            for record in &group.records {
                let badge = if record.conversion { "⇄" } else { "=" };
                list = push_row(
                    list,
                    badge,
                    record.question.clone(),
                    record.answer.clone(),
                    position,
                );
                position += 1;
            }
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }
}
