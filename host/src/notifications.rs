//! Windows notifications (toasts) forwarded to the phone.
//!
//! `UserNotificationListener` is polled once a second while at least one phone is connected to
//! `/notify` (polling is simpler and sturdier than `NotificationChanged`, which wants a registered
//! background task or package identity). Each poll is diffed against the ids already known; the
//! difference is broadcast as `Added` / `Removed` events. No notification text is ever logged.
//!
//! The diff, the text split, the time conversion and the app-id mapping are pure and unit tested;
//! the WinRT part sits behind the [`Source`] trait.

use crate::apps::AppInfo;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

pub const POLL_EVERY: Duration = Duration::from_secs(1);
/// Keep the bodies the phone has to carry sane.
const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notif {
    pub id: u32,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    /// Unix time, milliseconds.
    pub time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifEvent {
    Added(Notif),
    Removed(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Allowed,
    Denied,
    Unspecified,
    /// The API could not be used at all from this process.
    Unavailable,
}

impl Access {
    pub fn wire(self) -> &'static str {
        match self {
            Access::Allowed => "allowed",
            Access::Denied => "denied",
            Access::Unspecified => "unspecified",
            Access::Unavailable => "unavailable",
        }
    }
}

/// Where notifications come from (Windows in production, a fake in tests).
pub trait Source: Send + Sync + 'static {
    fn access(&self) -> Access;
    /// Shows Windows' one-time permission prompt (only called by the host itself at start).
    fn request_access(&self) -> Access;
    fn fetch(&self) -> Result<Vec<Notif>, String>;
    fn remove(&self, id: u32) -> Result<(), String>;
    /// Logo bytes (an image file, usually PNG) of an app seen in a fetched notification.
    fn logo(&self, app_id: &str) -> Option<Vec<u8>>;
}

// ---------------------------------------------------------------------------------------
// pure helpers

/// New notifications (in `current`'s order) and ids that disappeared.
pub fn diff(known: &HashSet<u32>, current: &[Notif]) -> (Vec<Notif>, Vec<u32>) {
    let added: Vec<Notif> = current.iter().filter(|n| !known.contains(&n.id)).cloned().collect();
    let now: HashSet<u32> = current.iter().map(|n| n.id).collect();
    let mut removed: Vec<u32> = known.iter().copied().filter(|id| !now.contains(id)).collect();
    removed.sort_unstable();
    (added, removed)
}

/// First text element = title, the rest joined by newlines = body.
pub fn split_texts(texts: &[String]) -> (String, String) {
    let clean: Vec<&str> = texts.iter().map(|s| s.trim()).collect();
    let title = clean.first().copied().unwrap_or("").to_string();
    let body = clean.iter().skip(1).filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
    (clamp(title, MAX_TITLE), clamp(body, MAX_BODY))
}

fn clamp(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('\u{2026}');
    t
}

/// Windows `DateTime.UniversalTime` (100 ns since 1601-01-01) to Unix milliseconds.
pub fn windows_time_to_unix_ms(universal: i64) -> i64 {
    (universal - 116_444_736_000_000_000) / 10_000
}

/// Notifications made by Penlink itself are not forwarded.
pub fn is_own(app_id: &str, app_name: &str) -> bool {
    app_id.to_ascii_lowercase().contains("penlink") || app_name.trim().eq_ignore_ascii_case("penlink")
}

/// The `/apps` id that opens the app behind a notification: the packaged app with the same
/// AUMID, else a catalogue entry with the same display name.
pub fn map_app(apps: &[AppInfo], app_id: &str, app_name: &str) -> Option<String> {
    if let Some(a) = apps.iter().find(|a| a.aumid.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(app_id))) {
        return Some(a.id.clone());
    }
    if !app_name.trim().is_empty() {
        if let Some(a) = apps.iter().find(|a| a.name.trim().eq_ignore_ascii_case(app_name.trim())) {
            return Some(a.id.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------------------
// the notifier

#[derive(Default)]
struct State {
    known: HashSet<u32>,
    primed: bool,
    last_error: Option<String>,
}

pub struct Notifier {
    src: Arc<dyn Source>,
    state: Mutex<State>,
    tx: broadcast::Sender<NotifEvent>,
    clients: AtomicUsize,
}

/// One connected phone: events plus the ids that existed when it connected. Dropping it
/// lets the poller go idle when it was the last one.
pub struct Subscription {
    pub events: broadcast::Receiver<NotifEvent>,
    pub ids: Vec<u32>,
    pub access: Access,
    owner: Arc<Notifier>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.owner.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Notifier {
    pub fn new(src: Arc<dyn Source>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self { src, state: Mutex::new(State::default()), tx, clients: AtomicUsize::new(0) })
    }

    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }

    async fn fetch(&self) -> Result<Vec<Notif>, String> {
        let src = self.src.clone();
        match tokio::task::spawn_blocking(move || src.fetch()).await {
            Ok(Ok(v)) => Ok(v.into_iter().filter(|n| !is_own(&n.app_id, &n.app_name)).collect()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn subscribe(self: &Arc<Self>) -> Subscription {
        let mut st = self.state.lock().await;
        self.clients.fetch_add(1, Ordering::SeqCst);
        if !st.primed {
            match self.fetch().await {
                Ok(v) => {
                    st.known = v.iter().map(|n| n.id).collect();
                    st.last_error = None;
                }
                Err(e) => {
                    tracing::warn!("cannot read Windows notifications: {e}");
                    st.known.clear();
                    st.last_error = Some(e);
                }
            }
            st.primed = true;
        }
        let access = if st.last_error.is_some() { Access::Unavailable } else { self.src.access() };
        let mut ids: Vec<u32> = st.known.iter().copied().collect();
        ids.sort_unstable();
        Subscription { events: self.tx.subscribe(), ids, access, owner: self.clone() }
    }

    /// One poll: fetch, diff, broadcast. Idle (and forgetting everything) with no phone connected.
    pub async fn poll_once(&self) {
        let mut st = self.state.lock().await;
        if self.clients() == 0 {
            st.known.clear();
            st.primed = false;
            return;
        }
        if !st.primed {
            return;
        }
        match self.fetch().await {
            Ok(cur) => {
                if st.last_error.take().is_some() {
                    tracing::info!("Windows notifications are readable again");
                }
                let (added, removed) = diff(&st.known, &cur);
                st.known = cur.iter().map(|n| n.id).collect();
                for n in added {
                    let _ = self.tx.send(NotifEvent::Added(n));
                }
                for id in removed {
                    let _ = self.tx.send(NotifEvent::Removed(id));
                }
            }
            Err(e) => {
                if st.last_error.as_deref() != Some(e.as_str()) {
                    tracing::warn!("cannot read Windows notifications: {e}");
                }
                st.last_error = Some(e);
            }
        }
    }

    pub fn spawn_poller(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(POLL_EVERY);
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                t.tick().await;
                me.poll_once().await;
            }
        })
    }

    /// Dismiss one notification on the PC (the poll then reports it as removed).
    pub async fn dismiss(&self, id: u32) {
        let src = self.src.clone();
        match tokio::task::spawn_blocking(move || src.remove(id)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::debug!("dismiss {id} failed: {e}"),
            Err(e) => tracing::debug!("dismiss task failed: {e}"),
        }
    }

    pub fn logo(&self, app_id: &str) -> Option<Vec<u8>> {
        self.src.logo(app_id)
    }

    /// First-run permission handling with plain console lines. Blocking (runs the Windows prompt).
    pub fn ensure_access_blocking(&self) -> Access {
        let mut a = self.src.access();
        match a {
            Access::Allowed => {}
            Access::Unspecified => {
                println!("Penlink forwards your Windows notifications to the phone. Windows will ask once for");
                println!("permission to let Penlink read notifications; choose Allow.");
                a = self.src.request_access();
                if a != Access::Allowed {
                    print_access_help(a);
                }
            }
            Access::Denied | Access::Unavailable => print_access_help(a),
        }
        a
    }
}

fn print_access_help(a: Access) {
    println!(
        "Notification forwarding is off ({}). To turn it on: Windows Settings > Privacy & security > Notifications > \"Let apps access your notifications\" (or Allow for Penlink), then restart the host.",
        a.wire()
    );
}

// ---------------------------------------------------------------------------------------
// Windows implementation

pub struct WinSource {
    logos: std::sync::Mutex<HashMap<String, Vec<u8>>>,
}

impl WinSource {
    pub fn new() -> Self {
        Self { logos: std::sync::Mutex::new(HashMap::new()) }
    }
}

impl Default for WinSource {
    fn default() -> Self {
        Self::new()
    }
}

mod win {
    use super::*;
    use windows::core::Result as WResult;
    use windows::Foundation::Size;
    use windows::Storage::Streams::{DataReader, RandomAccessStreamReference};
    use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
    use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

    pub fn access_of(s: UserNotificationListenerAccessStatus) -> Access {
        match s {
            UserNotificationListenerAccessStatus::Allowed => Access::Allowed,
            UserNotificationListenerAccessStatus::Denied => Access::Denied,
            _ => Access::Unspecified,
        }
    }

    pub fn access() -> Access {
        match UserNotificationListener::Current().and_then(|l| l.GetAccessStatus()) {
            Ok(s) => access_of(s),
            Err(_) => Access::Unavailable,
        }
    }

    pub fn request() -> Access {
        // Own thread so the blocking wait cannot stall the runtime.
        let r = std::thread::spawn(|| -> WResult<Access> {
            let l = UserNotificationListener::Current()?;
            Ok(access_of(l.RequestAccessAsync()?.join()?))
        })
        .join();
        match r {
            Ok(Ok(a)) => a,
            _ => Access::Unavailable,
        }
    }

    fn one(n: &UserNotification, logos: &std::sync::Mutex<HashMap<String, Vec<u8>>>) -> WResult<Notif> {
        let info = n.AppInfo()?;
        let app_id = info.AppUserModelId()?.to_string();
        let disp = info.DisplayInfo()?;
        let app_name = disp.DisplayName()?.to_string();
        let mut texts = Vec::new();
        if let Ok(vis) = n.Notification().and_then(|x| x.Visual()) {
            if let Ok(b) = vis.GetBinding(&KnownNotificationBindings::ToastGeneric()?) {
                for t in b.GetTextElements()? {
                    texts.push(t.Text()?.to_string());
                }
            }
        }
        let (title, body) = split_texts(&texts);
        if !logos.lock().unwrap().contains_key(&app_id) {
            // An app without a readable logo is remembered too (empty), so it is not retried every second.
            let bytes = disp.GetLogo(Size { Width: 64.0, Height: 64.0 }).and_then(|r| read_logo(&r)).unwrap_or_default();
            logos.lock().unwrap().insert(app_id.clone(), bytes);
        }
        Ok(Notif {
            id: n.Id()?,
            app_id,
            app_name,
            title,
            body,
            time_ms: windows_time_to_unix_ms(n.CreationTime()?.UniversalTime),
        })
    }

    fn read_logo(r: &RandomAccessStreamReference) -> WResult<Vec<u8>> {
        let stream = r.OpenReadAsync()?.join()?;
        let size64 = stream.Size()?;
        if size64 == 0 || size64 > 4 * 1024 * 1024 {
            return Ok(Vec::new());
        }
        let size = size64 as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.join()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;
        Ok(buf)
    }

    pub fn fetch(logos: &std::sync::Mutex<HashMap<String, Vec<u8>>>) -> Result<Vec<Notif>, String> {
        let l = UserNotificationListener::Current().map_err(|e| e.to_string())?;
        let list = l
            .GetNotificationsAsync(NotificationKinds::Toast)
            .and_then(|op| op.join())
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for n in list {
            // One odd notification must not hide the others.
            match one(&n, logos) {
                Ok(x) => out.push(x),
                Err(e) => tracing::debug!("skipped one notification: {e}"),
            }
        }
        Ok(out)
    }

    pub fn remove(id: u32) -> Result<(), String> {
        UserNotificationListener::Current().and_then(|l| l.RemoveNotification(id)).map_err(|e| e.to_string())
    }
}

impl Source for WinSource {
    fn access(&self) -> Access {
        win::access()
    }
    fn request_access(&self) -> Access {
        win::request()
    }
    fn fetch(&self) -> Result<Vec<Notif>, String> {
        if win::access() == Access::Denied {
            return Err("access to notifications is denied in Windows settings".into());
        }
        win::fetch(&self.logos)
    }
    fn remove(&self, id: u32) -> Result<(), String> {
        win::remove(id)
    }
    fn logo(&self, app_id: &str) -> Option<Vec<u8>> {
        self.logos.lock().unwrap().get(app_id).filter(|b| !b.is_empty()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn n(id: u32, app: &str, title: &str) -> Notif {
        Notif { id, app_id: app.into(), app_name: app.into(), title: title.into(), body: String::new(), time_ms: 0 }
    }

    struct Fake {
        list: StdMutex<Result<Vec<Notif>, String>>,
        removed: StdMutex<Vec<u32>>,
    }
    impl Source for Fake {
        fn access(&self) -> Access {
            Access::Allowed
        }
        fn request_access(&self) -> Access {
            Access::Allowed
        }
        fn fetch(&self) -> Result<Vec<Notif>, String> {
            self.list.lock().unwrap().clone()
        }
        fn remove(&self, id: u32) -> Result<(), String> {
            self.removed.lock().unwrap().push(id);
            Ok(())
        }
        fn logo(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }
    fn fake(v: Vec<Notif>) -> Arc<Fake> {
        Arc::new(Fake { list: StdMutex::new(Ok(v)), removed: StdMutex::new(vec![]) })
    }

    #[test]
    fn diff_reports_added_and_removed() {
        let known: HashSet<u32> = [1, 2, 3].into();
        let (a, r) = diff(&known, &[n(2, "x", "b"), n(4, "x", "d"), n(5, "x", "e")]);
        assert_eq!(a.iter().map(|x| x.id).collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!(r, vec![1, 3]);
        let (a, r) = diff(&known, &[n(1, "x", ""), n(2, "x", ""), n(3, "x", "")]);
        assert!(a.is_empty() && r.is_empty());
    }

    #[test]
    fn splits_title_and_body() {
        let t = |v: &[&str]| split_texts(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(t(&["Claude", "Task finished", " 3 files "]), ("Claude".into(), "Task finished\n3 files".into()));
        assert_eq!(t(&["Only title"]), ("Only title".into(), String::new()));
        assert_eq!(t(&[]), (String::new(), String::new()));
        let long = "x".repeat(5000);
        let (_, body) = t(&["t", &long]);
        assert_eq!(body.chars().count(), MAX_BODY);
        assert!(body.ends_with('\u{2026}'));
    }

    #[test]
    fn converts_windows_time() {
        assert_eq!(windows_time_to_unix_ms(116_444_736_000_000_000), 0);
        assert_eq!(windows_time_to_unix_ms(116_444_736_000_000_000 + 1_700_000_000_000 * 10_000), 1_700_000_000_000);
    }

    #[test]
    fn own_notifications_are_recognised() {
        assert!(is_own("dev.Penlink.Host", "x"));
        assert!(is_own("whatever", "Penlink"));
        assert!(!is_own("Claude_abc!App", "Claude"));
    }

    #[test]
    fn maps_notification_app_to_catalogue_entry() {
        let mk = |id: &str, name: &str, aumid: Option<&str>| AppInfo {
            id: id.into(),
            name: name.into(),
            lnk: Default::default(),
            target: String::new(),
            exe_name: None,
            aumid: aumid.map(|s| s.into()),
        };
        let apps = vec![mk("a1", "Claude", Some("Claude_pzs8!Claude")), mk("b2", "Notepad", None)];
        assert_eq!(map_app(&apps, "claude_pzs8!claude", "x").as_deref(), Some("a1"));
        assert_eq!(map_app(&apps, "C:\\x\\notepad.exe", "NOTEPAD").as_deref(), Some("b2"));
        assert_eq!(map_app(&apps, "nope", "Nothing"), None);
    }

    #[tokio::test]
    async fn broadcasts_new_and_removed_and_skips_baseline_and_own() {
        let f = fake(vec![n(1, "a", "old"), n(2, "dev.penlink", "mine")]);
        let nf = Notifier::new(f.clone());
        let mut sub = nf.subscribe().await;
        assert_eq!(sub.ids, vec![1]);
        nf.poll_once().await;
        assert!(sub.events.try_recv().is_err());

        *f.list.lock().unwrap() = Ok(vec![n(1, "a", "old"), n(3, "b", "new"), n(4, "penlink", "own")]);
        nf.poll_once().await;
        assert_eq!(sub.events.try_recv().unwrap(), NotifEvent::Added(n(3, "b", "new")));
        assert!(sub.events.try_recv().is_err());

        *f.list.lock().unwrap() = Ok(vec![n(3, "b", "new")]);
        nf.poll_once().await;
        assert_eq!(sub.events.try_recv().unwrap(), NotifEvent::Removed(1));
    }

    #[tokio::test]
    async fn idle_without_clients_and_rebaselines() {
        let f = fake(vec![n(1, "a", "x")]);
        let nf = Notifier::new(f.clone());
        drop(nf.subscribe().await);
        assert_eq!(nf.clients(), 0);
        nf.poll_once().await;
        *f.list.lock().unwrap() = Ok(vec![n(1, "a", "x"), n(9, "a", "while away")]);
        let sub = nf.subscribe().await;
        assert_eq!(sub.ids, vec![1, 9]);
    }

    #[tokio::test]
    async fn read_error_marks_unavailable_then_recovers() {
        let f = Arc::new(Fake { list: StdMutex::new(Err("denied".into())), removed: StdMutex::new(vec![]) });
        let nf = Notifier::new(f.clone());
        let mut sub = nf.subscribe().await;
        assert_eq!(sub.access, Access::Unavailable);
        nf.poll_once().await;
        *f.list.lock().unwrap() = Ok(vec![n(1, "a", "x")]);
        nf.poll_once().await;
        assert_eq!(sub.events.try_recv().unwrap(), NotifEvent::Added(n(1, "a", "x")));
    }

    #[tokio::test]
    async fn dismiss_calls_the_source() {
        let f = fake(vec![]);
        let nf = Notifier::new(f.clone());
        nf.dismiss(7).await;
        assert_eq!(*f.removed.lock().unwrap(), vec![7]);
    }
}
