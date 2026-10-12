//! Rendering tests on the in-memory test backend.

#[cfg(test)]
mod tests {
    use bili_api::{LiveStatus, RoomID, RoomInfo, UID};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::{App, View};
    use crate::room::RoomState;
    use crate::ui;

    fn fake_room(input: &str) -> RoomState {
        let mut room = RoomState::pending(input.to_string());
        room.resolved = true;
        room.room_id = Some(RoomID(1234));
        room.info = Some(RoomInfo {
            room_id: RoomID(1234),
            short_id: 6,
            uid: UID(42),
            title: "my stream".into(),
            parent_area_name: String::new(),
            live_status: LiveStatus::LIVE,
            online: 42,
        });
        room.up_name = Some("Streamer".into());
        room
    }

    #[test]
    fn grid_renders_room_cell() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None);
        app.rooms.push(fake_room("6"));
        app.view = View::Grid;
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("my stream"), "title missing:\n{content}");
        assert!(content.contains("Streamer"), "owner missing:\n{content}");
        assert!(content.contains("42"), "online missing:\n{content}");
    }

    #[test]
    fn prompt_shows_input_text() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None);
        app.view = View::Prompt;
        app.prompt = "7734200".into();
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("7734200"), "input text missing:\n{content}");
        assert!(content.contains("Enter add"), "hint missing:\n{content}");
    }

    #[test]
    fn focus_wraps_at_edges() {
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None);
        app.rooms.push(fake_room("6"));
        app.rooms.push(fake_room("7"));
        app.last_width = 80;
        assert_eq!(app.focused, 0);

        app.move_focus(1, 0);
        assert_eq!(app.focused, 1);

        // l on the last room wraps back to the first.
        app.move_focus(1, 0);
        assert_eq!(app.focused, 0);

        // h on the first room wraps to the last.
        app.move_focus(-1, 0);
        assert_eq!(app.focused, 1);
    }

    #[test]
    fn empty_grid_shows_hint() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None);
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("no rooms"));
        assert!(content.contains("press g"));
    }
}
