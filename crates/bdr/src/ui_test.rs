//! Rendering tests on the in-memory test backend.

#[cfg(test)]
mod tests {
    use bili_api::{LiveStatus, RoomID, RoomInfo, UID};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
        let mut app = App::new(api, None, None, None);
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
        let mut app = App::new(api, None, None, None);
        app.view = View::Prompt;
        app.prompt = "7734200".into();
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("7734200"),
            "input text missing:\n{content}"
        );
        assert!(content.contains("Enter add"), "hint missing:\n{content}");
    }

    #[test]
    fn focus_wraps_at_edges() {
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None, None);
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
        let mut app = App::new(api, None, None, None);
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("no rooms"));
        assert!(content.contains("press g"));
    }

    fn following_home_fixture() -> bili_api::LiveHome {
        use bili_api::{LiveHome, LiveRoomEntry, RoomID, UID};
        fn entry(room: u64, uname: &str, online: u64) -> LiveRoomEntry {
            LiveRoomEntry {
                room_id: RoomID(room),
                uid: UID(100 + room),
                uname: uname.into(),
                title: "live title".into(),
                online,
            }
        }
        LiveHome {
            followed: vec![entry(1, "liver a", 10), entry(2, "liver b", 20)],
            recommended: vec![entry(3, "liver c", 30)],
        }
    }

    #[test]
    fn following_view_renders_rooms() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None, Some(bili_api::UID(42)));
        app.following = Some(crate::app::Following {
            home: Some(following_home_fixture()),
            sel: 0,
            error: None,
        });
        app.view = View::Following;
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("liver a"),
            "followed room missing:\n{content}"
        );
        assert!(
            content.contains("liver c"),
            "recommended room missing:\n{content}"
        );
        assert!(content.contains("following"), "title missing:\n{content}");
    }

    #[test]
    fn following_view_shows_login_hint_when_logged_out() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None, None);
        app.following = Some(crate::app::Following {
            home: Some(following_home_fixture()),
            sel: 0,
            error: None,
        });
        app.view = View::Following;
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("not logged in"),
            "hint missing:\n{content}"
        );
    }

    #[test]
    fn login_view_renders_qr_and_status() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None, None);
        app.login = Some(crate::app::LoginSession {
            key: Some("k".into()),
            url: Some("https://account.bilibili.com/h5/scan-web?qrcode_key=abcd".into()),
            status: bili_api::QrcodeStatus::Waiting,
            next_poll: None,
            error: None,
        });
        app.view = View::Login;
        ui::draw(&mut terminal, &mut app).unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("waiting for the scan"),
            "status missing:\n{content}"
        );
        assert!(
            content.contains('\u{2588}'),
            "QR blocks missing:\n{content}"
        );
    }

    #[test]
    #[ignore = "writes real credentials to the user config dir"]
    fn login_success_rebuilds_client_and_returns_to_grid() {
        let api = bili_api::Client::new();
        let mut app = App::new(api, None, None, None);
        app.login = Some(crate::app::LoginSession::default());
        app.view = View::Login;
        app.apply_login(crate::app::LoginMsg::Polled {
            status: bili_api::QrcodeStatus::Success,
            cookies: vec![
                ("SESSDATA".into(), "s".into()),
                ("bili_jct".into(), "j".into()),
                ("DedeUserID".into(), "99".into()),
            ],
            refresh_token: None,
        });
        assert_eq!(app.view, View::Grid);
        assert_eq!(app.logged_in, Some(bili_api::UID(99)));
        assert!(app.api().cookie().contains("SESSDATA=s"));
        assert!(app.login.is_none());
    }

    // `confirm_following` opens the room, which spawns the open
    // pipeline. Run under a runtime so the spawns work; the spawned
    // network tasks finish in the background and do not affect the
    // assertions here.
    #[tokio::test]
    async fn confirm_following_opens_the_selected_room() {
        let api = bili_api::Client::from_config(bili_api::ClientConfig::builder().build());
        let mut app = App::new(api, None, None, None);
        app.following = Some(crate::app::Following {
            home: Some(following_home_fixture()),
            sel: 1,
            error: None,
        });
        app.confirm_following();
        assert_eq!(app.view, View::Grid);
        assert_eq!(app.rooms.len(), 1);
        assert_eq!(app.rooms[0].input, "2");
    }
}
