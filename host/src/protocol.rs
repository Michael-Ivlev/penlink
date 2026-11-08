//! Wire protocol (see SPEC.md "Protocol"). Pure serde types, no I/O.

use serde::{Deserialize, Serialize};

pub const VERSION: &str = "0.1.0";
pub const DEFAULT_PORT: u16 = 47800;

fn default_scale() -> f64 {
    1.0
}

/// Phone -> host, WebSocket text frames.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMsg {
    #[serde(rename_all = "camelCase")]
    Open {
        app_id: String,
        width: u32,
        height: u32,
        #[serde(default = "default_scale")]
        scale: f64,
        /// Re-open after a reconnect: attach to the app's existing window, never launch it.
        #[serde(default)]
        attach: bool,
        /// `"sharp"` (monitor = phone pixels) or `"fast"` (half). A client that sends this or
        /// `uiScale` gets a virtual monitor when the driver is installed.
        #[serde(default)]
        quality: Option<String>,
        /// UI size in percent (100..500). Without it `scale * 100` is used.
        #[serde(default)]
        ui_scale: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Answer { session_id: String, sdp: String },
    #[serde(rename_all = "camelCase")]
    Ice {
        session_id: String,
        candidate: String,
        #[serde(default)]
        sdp_mid: Option<String>,
        #[serde(default)]
        sdp_m_line_index: Option<u16>,
    },
    #[serde(rename_all = "camelCase")]
    Resize {
        session_id: String,
        width: u32,
        height: u32,
        #[serde(default = "default_scale")]
        scale: f64,
        #[serde(default)]
        quality: Option<String>,
        #[serde(default)]
        ui_scale: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Close { session_id: String },
    #[serde(rename_all = "camelCase")]
    Keyframe { session_id: String },
}

/// What the video shows (`window` message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    #[default]
    Window,
    Monitor,
}

/// Host -> phone, WebSocket text frames.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMsg {
    #[serde(rename_all = "camelCase")]
    Opened {
        session_id: String,
        app_id: String,
        title: String,
    },
    /// Window title became known / changed (optional, after `opened`).
    #[serde(rename_all = "camelCase")]
    Title { session_id: String, title: String },
    /// The real size of the PC window (client area, physical pixels) after an open or resize.
    /// `fitted`: it was shrunk to fit the PC screen, so the video is upscaled.
    #[serde(rename_all = "camelCase")]
    Window {
        session_id: String,
        width: i32,
        height: i32,
        fitted: bool,
        /// What is being streamed: the app's window, or a virtual monitor.
        #[serde(default)]
        mode: StreamMode,
        /// Monitor mode: the DPI percentage Windows really applied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dpi: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Offer { session_id: String, sdp: String },
    #[serde(rename_all = "camelCase")]
    Ice {
        session_id: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
    },
    #[serde(rename_all = "camelCase")]
    Closed { session_id: String, reason: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub code: String,
    #[serde(default)]
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub token: String,
    pub host_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoResponse {
    pub host_name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PenAction {
    Hover,
    Down,
    Move,
    Up,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TouchAction {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseAction {
    Move,
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    Down,
    Up,
}

/// Data channel `input` messages.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum InputMsg {
    Pen {
        a: PenAction,
        x: f64,
        y: f64,
        #[serde(default)]
        p: f64,
        #[serde(default)]
        tx: f64,
        #[serde(default)]
        ty: f64,
        #[serde(default)]
        b: u32,
        #[serde(default)]
        e: bool,
    },
    Touch {
        a: TouchAction,
        id: u32,
        x: f64,
        y: f64,
    },
    Mouse {
        a: MouseAction,
        x: f64,
        y: f64,
        #[serde(default)]
        btn: Button,
    },
    Mrel {
        dx: f64,
        dy: f64,
    },
    Wheel {
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    Key {
        a: KeyAction,
        vk: u16,
    },
    Text {
        s: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_open() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"open","appId":"abc","width":1440,"height":3120,"scale":2.0}"#,
        )
        .unwrap();
        assert_eq!(
            m,
            ClientMsg::Open {
                app_id: "abc".into(),
                width: 1440,
                height: 3120,
                scale: 2.0,
                attach: false,
                quality: None,
                ui_scale: None
            }
        );
    }

    #[test]
    fn open_scale_defaults_to_one_and_attach_to_false() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"open","appId":"a","width":10,"height":20}"#).unwrap();
        assert!(matches!(m, ClientMsg::Open { scale, attach: false, .. } if scale == 1.0));
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"open","appId":"a","width":10,"height":20,"attach":true}"#).unwrap();
        assert!(matches!(m, ClientMsg::Open { attach: true, .. }));
    }

    #[test]
    fn open_and_resize_carry_quality_and_ui_scale() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"open","appId":"a","width":1440,"height":3120,"scale":3.0,"quality":"fast","uiScale":275}"#,
        )
        .unwrap();
        assert!(matches!(
            m,
            ClientMsg::Open { quality: Some(ref q), ui_scale: Some(u), .. } if q == "fast" && u == 275.0
        ));
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"resize","sessionId":"s","width":1440,"height":1500,"scale":3.0,"quality":"sharp","uiScale":300}"#,
        )
        .unwrap();
        assert!(matches!(m, ClientMsg::Resize { quality: Some(_), ui_scale: Some(u), .. } if u == 300.0));
        // an old client without the fields still works
        let m: ClientMsg = serde_json::from_str(r#"{"type":"resize","sessionId":"s","width":1,"height":2}"#).unwrap();
        assert!(matches!(m, ClientMsg::Resize { quality: None, ui_scale: None, .. }));
    }

    #[test]
    fn parses_ice_with_camel_case_fields() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"ice","sessionId":"s1","candidate":"candidate:1 1 udp 1 1.2.3.4 5 typ host","sdpMid":"0","sdpMLineIndex":0}"#,
        )
        .unwrap();
        assert_eq!(
            m,
            ClientMsg::Ice {
                session_id: "s1".into(),
                candidate: "candidate:1 1 udp 1 1.2.3.4 5 typ host".into(),
                sdp_mid: Some("0".into()),
                sdp_m_line_index: Some(0)
            }
        );
    }

    #[test]
    fn parses_answer_resize_close_keyframe() {
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"answer","sessionId":"s","sdp":"v=0"}"#).unwrap(),
            ClientMsg::Answer { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(
                r#"{"type":"resize","sessionId":"s","width":3120,"height":1440,"scale":2.0}"#
            )
            .unwrap(),
            ClientMsg::Resize { width: 3120, height: 1440, .. }
        ));
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"close","sessionId":"s"}"#).unwrap(),
            ClientMsg::Close { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"keyframe","sessionId":"s"}"#).unwrap(),
            ClientMsg::Keyframe { .. }
        ));
    }

    #[test]
    fn unknown_type_is_an_error() {
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn server_messages_match_spec_json() {
        let v = serde_json::to_value(ServerMsg::Opened {
            session_id: "s".into(),
            app_id: "a".into(),
            title: "Untitled - Notepad".into(),
        })
        .unwrap();
        assert_eq!(v, json!({"type":"opened","sessionId":"s","appId":"a","title":"Untitled - Notepad"}));

        let v = serde_json::to_value(ServerMsg::Offer { session_id: "s".into(), sdp: "v=0".into() }).unwrap();
        assert_eq!(v, json!({"type":"offer","sessionId":"s","sdp":"v=0"}));

        let v = serde_json::to_value(ServerMsg::Ice {
            session_id: "s".into(),
            candidate: "c".into(),
            sdp_mid: Some("0".into()),
            sdp_m_line_index: Some(1),
        })
        .unwrap();
        assert_eq!(v, json!({"type":"ice","sessionId":"s","candidate":"c","sdpMid":"0","sdpMLineIndex":1}));

        let v = serde_json::to_value(ServerMsg::Title { session_id: "s".into(), title: "T".into() }).unwrap();
        assert_eq!(v, json!({"type":"title","sessionId":"s","title":"T"}));
        let v = serde_json::to_value(ServerMsg::Window {
            session_id: "s".into(),
            width: 812,
            height: 1760,
            fitted: true,
            mode: StreamMode::Window,
            dpi: None,
        })
        .unwrap();
        assert_eq!(v, json!({"type":"window","sessionId":"s","width":812,"height":1760,"fitted":true,"mode":"window"}));
        let v = serde_json::to_value(ServerMsg::Window {
            session_id: "s".into(),
            width: 1440,
            height: 3120,
            fitted: false,
            mode: StreamMode::Monitor,
            dpi: Some(300),
        })
        .unwrap();
        assert_eq!(v, json!({"type":"window","sessionId":"s","width":1440,"height":3120,"fitted":false,"mode":"monitor","dpi":300}));
        // an old host message without mode still parses (as window mode)
        let m: ServerMsg = serde_json::from_str(r#"{"type":"window","sessionId":"s","width":1,"height":2,"fitted":false}"#).unwrap();
        assert!(matches!(m, ServerMsg::Window { mode: StreamMode::Window, dpi: None, .. }));
        let v = serde_json::to_value(ServerMsg::Closed { session_id: "s".into(), reason: "app exited".into() }).unwrap();
        assert_eq!(v, json!({"type":"closed","sessionId":"s","reason":"app exited"}));

        let v = serde_json::to_value(ServerMsg::Error { message: "bad".into() }).unwrap();
        assert_eq!(v, json!({"type":"error","message":"bad"}));
    }

    #[test]
    fn http_bodies_match_spec() {
        let r: PairRequest = serde_json::from_str(r#"{"code":"123456","deviceName":"S26 Ultra"}"#).unwrap();
        assert_eq!(r.code, "123456");
        assert_eq!(r.device_name, "S26 Ultra");
        let v = serde_json::to_value(PairResponse { token: "t".into(), host_name: "H".into() }).unwrap();
        assert_eq!(v, json!({"token":"t","hostName":"H"}));
        let v = serde_json::to_value(InfoResponse { host_name: "H".into(), version: VERSION.into() }).unwrap();
        assert_eq!(v, json!({"hostName":"H","version":"0.1.0"}));
        let v = serde_json::to_value(vec![AppEntry { id: "i".into(), name: "n".into() }]).unwrap();
        assert_eq!(v, json!([{"id":"i","name":"n"}]));
    }

    #[test]
    fn parses_pen() {
        let m: InputMsg = serde_json::from_str(
            r#"{"t":"pen","a":"hover","x":0.5,"y":0.5,"p":0.6,"tx":10,"ty":-5,"b":1,"e":true}"#,
        )
        .unwrap();
        assert_eq!(
            m,
            InputMsg::Pen { a: PenAction::Hover, x: 0.5, y: 0.5, p: 0.6, tx: 10.0, ty: -5.0, b: 1, e: true }
        );
        // optional fields default
        let m: InputMsg = serde_json::from_str(r#"{"t":"pen","a":"leave","x":0,"y":0}"#).unwrap();
        assert!(matches!(m, InputMsg::Pen { a: PenAction::Leave, p, b: 0, e: false, .. } if p == 0.0));
    }

    #[test]
    fn parses_touch_mouse_mrel_wheel_key_text() {
        let m: InputMsg = serde_json::from_str(r#"{"t":"touch","a":"down","id":3,"x":0.1,"y":0.2}"#).unwrap();
        assert_eq!(m, InputMsg::Touch { a: TouchAction::Down, id: 3, x: 0.1, y: 0.2 });
        let m: InputMsg =
            serde_json::from_str(r#"{"t":"mouse","a":"down","x":0.5,"y":0.5,"btn":"right"}"#).unwrap();
        assert_eq!(m, InputMsg::Mouse { a: MouseAction::Down, x: 0.5, y: 0.5, btn: Button::Right });
        let m: InputMsg = serde_json::from_str(r#"{"t":"mouse","a":"move","x":0.5,"y":0.5}"#).unwrap();
        assert!(matches!(m, InputMsg::Mouse { btn: Button::Left, .. }));
        let m: InputMsg = serde_json::from_str(r#"{"t":"mrel","dx":3.5,"dy":-2}"#).unwrap();
        assert_eq!(m, InputMsg::Mrel { dx: 3.5, dy: -2.0 });
        let m: InputMsg = serde_json::from_str(r#"{"t":"wheel","dx":0,"dy":-120}"#).unwrap();
        assert_eq!(m, InputMsg::Wheel { dx: 0.0, dy: -120.0 });
        let m: InputMsg = serde_json::from_str(r#"{"t":"key","a":"down","vk":17}"#).unwrap();
        assert_eq!(m, InputMsg::Key { a: KeyAction::Down, vk: 17 });
        let m: InputMsg = serde_json::from_str(r#"{"t":"text","s":"héllo"}"#).unwrap();
        assert_eq!(m, InputMsg::Text { s: "héllo".into() });
    }
}
