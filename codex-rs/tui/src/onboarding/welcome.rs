use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::WidgetRef;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::tui::FrameRequester;
use crossterm::event::KeyEvent;

use super::onboarding_screen::StepState;

pub(crate) struct WelcomeWidget {
    pub is_logged_in: bool,
}

impl WelcomeWidget {
    pub(crate) fn new(is_logged_in: bool, _request_frame: FrameRequester) -> Self {
        Self { is_logged_in }
    }
}

impl WidgetRef for &WelcomeWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(vec![
            ">_ ".into(),
            "Welcome to Codex, OpenAI's command-line coding agent".bold(),
        ]);
        line.render(area, buf);
    }
}

impl StepStateProvider for WelcomeWidget {
    fn get_step_state(&self) -> StepState {
        match self.is_logged_in {
            true => StepState::Hidden,
            false => StepState::Complete,
        }
    }
}

impl KeyboardHandler for WelcomeWidget {
    fn handle_key_event(&mut self, _key_event: KeyEvent) {
        // Welcome page has no interactive elements yet.
    }
}

// no tests in main’s version
