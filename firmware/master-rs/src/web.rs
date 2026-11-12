//! The web page of the board: widget registry, layout, password, charts and the network form.
//!
//! Pure logic of the C++ master's `CRAB_WEB` part (ops 17..24 of the `net` import and the HTTP
//! routes of the page). The board glue only forwards HTTP requests to [`WebState::handle`], calls
//! [`WebState::net_op`] from the `net` import and [`WebState::tick`] from its loop, and keeps
//! what must survive a reset (layout, saved network form) in flash.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use crab_kernel::host::Mem;

/// Widgets of the page (the master's table has 32 rows).
pub const MAX_WIDGETS: usize = 32;
/// Text of a widget, bytes.
pub const MAX_TEXT: usize = 128;
/// Layout JSON, bytes (exclusive).
pub const MAX_LAYOUT: usize = 8192;
/// Fields of the network form: ssid, password, dhcp, ip, gateway, subnet, ap ssid, ap password,
/// mode, ap delay.
pub const NET_FIELDS: usize = 10;
/// Query keys of the network form, in the order of the fields.
const NET_KEYS: [&str; NET_FIELDS] = ["s", "p", "d", "i", "g", "m", "a", "b", "o", "w"];

pub const KIND_NUMBER: u8 = 0;
pub const KIND_TEXT: u8 = 1;
pub const KIND_TEXT_INPUT: u8 = 2;
pub const KIND_SECRET: u8 = 3;
pub const KIND_CHART: u8 = 4;

/// One widget of the page.
#[derive(Debug, Clone, Default)]
struct Widget {
    kind: u8,
    rw: bool,
    locked: bool,
    dec: u8,
    lo: f32,
    hi: f32,
    value: f32,
    /// Text kinds: up to 128 bytes.
    text: Vec<u8>,
    /// Chart: ring buffer of `hist.len()` points.
    hist: Vec<f32>,
    hpos: usize,
    hfill: usize,
    hlast: u32,
    /// Chart sampling period, ms.
    hperiod: u32,
}

impl Widget {
    fn has_text(&self) -> bool {
        matches!(self.kind, KIND_TEXT | KIND_TEXT_INPUT | KIND_SECRET)
    }
}

/// A request of the page, decoded by the board glue.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Query arguments, percent-decoded.
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// A GET request; handy for the glue and tests.
    pub fn get(path: &str, query: &[(&str, &str)]) -> Self {
        Self {
            method: "GET".to_string(),
            path: path.to_string(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Vec::new(),
        }
    }

    /// A request from the method, the URI as the server gives it (`/ui/set?i=1&v=a%20b`) and the body.
    pub fn from_uri(method: &str, uri: &str, body: Vec<u8>) -> Self {
        let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
        Self {
            method: method.to_string(),
            path: path.to_string(),
            query: query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap_or((p, ""));
                    (percent_decode(k), percent_decode(v))
                })
                .collect(),
            body,
        }
    }

    fn arg(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// `%XX` and `+` of a query part; bad escapes stay as they are, invalid UTF-8 is replaced.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An answer to give back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn new(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type,
            body: body.into(),
        }
    }

    /// The body as text (for tests and logs).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Technical data for the «Система» page, collected by the board glue.
#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub ip: String,
    pub mac: String,
    pub rssi: i32,
    pub uptime_s: u32,
    pub chip: String,
    pub cpu_mhz: u32,
    pub heap_free: u32,
    pub heap_total: u32,
    pub flash_size: u32,
    pub app_size: u32,
    pub app_free: u32,
    pub build: String,
    /// Wi-Fi scan for `/ui/scan`: "?" while it runs, else one `ssid<TAB>rssi` per line (at most
    /// 1800 bytes). The glue starts the scan when it sees [`WebState::scan_wanted`].
    pub scan: String,
}

/// The saved values of the network form, to keep in flash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetSaved {
    /// Hash of the defaults the module gave; saved values are dropped when it changes.
    pub sig: u32,
    pub fields: [String; NET_FIELDS],
}

/// FNV-1a, as the C++ master hashes the layout.
pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h = 2166136261u32;
    for &b in bytes {
        h = (h ^ u32::from(b)).wrapping_mul(16777619);
    }
    h
}

fn fnv_fields(fields: &[String]) -> u32 {
    let mut h = 2166136261u32;
    for f in fields {
        for &b in f.as_bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(16777619);
        }
        h = (h ^ 0xFF).wrapping_mul(16777619);
    }
    h
}

/// Arduino `String::toInt`: leading number, 0 if none.
fn to_int(s: &str) -> i32 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut end = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    s[..end]
        .parse::<i64>()
        .unwrap_or(0)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Arduino `String::toFloat`: leading number, 0 if none.
fn to_float(s: &str) -> f32 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let digits = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let mut end = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    end = digits(end);
    if b.get(end) == Some(&b'.') {
        end = digits(end + 1);
    }
    if matches!(b.get(end), Some(b'e' | b'E')) {
        let mut e = end + 1;
        if matches!(b.get(e), Some(b'+' | b'-')) {
            e += 1;
        }
        let after = digits(e);
        if after > e {
            end = after;
        }
    }
    s[..end].parse::<f32>().unwrap_or(0.0)
}

/// Cuts a text at its first NUL and at `MAX_TEXT` bytes.
fn clip_text(bytes: &[u8]) -> Vec<u8> {
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..n.min(MAX_TEXT)].to_vec()
}

/// The whole state of the page.
pub struct WebState {
    page: &'static str,
    widgets: Vec<Widget>,
    layout: Vec<u8>,
    password: String,
    mask: i32,
    system: bool,
    complete: bool,
    // network form
    net_on: bool,
    net_item: Option<usize>,
    net_fields: [String; NET_FIELDS],
    net_default_sig: u32,
    net_restored: Option<NetSaved>,
    net_changed: bool,
    net_dirty: bool,
    scan_wanted: bool,
}

impl WebState {
    /// `page` is the HTML of the page (`crab_blocks::device_page()`).
    pub fn new(page: &'static str) -> Self {
        Self {
            page,
            widgets: Vec::new(),
            layout: Vec::new(),
            password: String::new(),
            mask: 1,
            system: false,
            complete: false,
            net_on: false,
            net_item: None,
            net_fields: Default::default(),
            net_default_sig: 0,
            net_restored: None,
            net_changed: false,
            net_dirty: false,
            scan_wanted: false,
        }
    }

    // ---- what the board glue reads ----

    /// The module sent everything (op 17, code 63): the server may start.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Interfaces to serve on: bit 1 Wi-Fi, bit 2 Ethernet.
    pub fn interfaces(&self) -> i32 {
        self.mask
    }

    /// The «Система» page (and `/ui/info`) is on.
    pub fn system_page(&self) -> bool {
        self.system
    }

    /// The network form is on (`/ui/net` and `/ui/scan` exist).
    pub fn net_form(&self) -> bool {
        self.net_on
    }

    /// The layout and its hash, to store in flash; `None` when there is no layout.
    pub fn layout_for_save(&self) -> Option<(&[u8], u32)> {
        (!self.layout.is_empty()).then(|| (self.layout.as_slice(), fnv1a(&self.layout)))
    }

    /// Restores the layout kept in flash at start. Refused when the length or the hash do not
    /// fit, or when the module already gave a layout.
    pub fn restore_layout(&mut self, bytes: &[u8], hash: u32) -> bool {
        if !self.layout.is_empty()
            || bytes.is_empty()
            || bytes.len() >= MAX_LAYOUT
            || fnv1a(bytes) != hash
        {
            return false;
        }
        self.layout = bytes.to_vec();
        true
    }

    /// Restores the saved network form at start (before the module sends its defaults).
    pub fn restore_net(&mut self, saved: NetSaved) {
        self.net_restored = Some(saved);
    }

    /// The network form values to store, once after each change (`None` if nothing changed).
    pub fn take_net_save(&mut self) -> Option<NetSaved> {
        if !self.net_dirty {
            return None;
        }
        self.net_dirty = false;
        Some(NetSaved {
            sig: self.net_default_sig,
            fields: self.net_fields.clone(),
        })
    }

    /// Values of the network form: ssid, password, dhcp, ip, gateway, subnet, ap ssid, ap
    /// password, mode, ap delay.
    pub fn net_fields(&self) -> &[String; NET_FIELDS] {
        &self.net_fields
    }

    /// The page changed a field since the module last asked (op 23, code 10).
    pub fn net_changed(&self) -> bool {
        self.net_changed
    }

    /// DHCP chosen on the page.
    pub fn net_dhcp(&self) -> bool {
        self.net_fields[2] != "0"
    }

    /// Radio mode on the page: 0 client, 1 access point, 2 both, 3 client, then the point only
    /// without a connection.
    pub fn net_mode(&self) -> i32 {
        to_int(&self.net_fields[8])
    }

    /// Changes with the fields that need the radio or the address to be set again (not the mode
    /// and the delay), like the C++ `wnSign`.
    pub fn net_signature(&self) -> u32 {
        fnv_fields(&self.net_fields[..8])
    }

    /// The page asked for a Wi-Fi scan since the last call (the flag is cleared).
    pub fn scan_wanted(&mut self) -> bool {
        core::mem::take(&mut self.scan_wanted)
    }

    // ---- ops of the `net` import ----

    /// Ops 17, 19..24 of `net(op, a, b, c, d)`; other ops return 0 (they are not the page's).
    #[allow(clippy::too_many_arguments)]
    pub fn net_op(
        &mut self,
        op: i32,
        a: i32,
        b: i32,
        c: i32,
        d: i32,
        mem: &mut Mem,
        _now_ms: u32,
    ) -> i32 {
        let count = self.widgets.len();
        let idx = usize::try_from(a).ok().filter(|&i| i < count);
        match op {
            17 => {
                self.setup(a, b, c, d, mem);
            }
            19 => {
                if let Some(i) = idx {
                    self.widgets[i].value = f32::from_bits(b as u32);
                }
            }
            20 => {
                if let Some(i) = idx.filter(|&i| self.widgets[i].has_text()) {
                    if c >= 0 {
                        if let Some(t) = mem.read(b, c, MAX_TEXT) {
                            self.widgets[i].text = clip_text(&t);
                        }
                    }
                }
            }
            21 => {
                if let Some(i) = idx {
                    return self.widgets[i].value.to_bits() as i32;
                }
            }
            22 => {
                if let Some(i) = idx.filter(|&i| self.widgets[i].has_text()) {
                    if b >= 0 && c > 0 {
                        let k = self.widgets[i].text.len().min(c as usize - 1);
                        let mut out = self.widgets[i].text[..k].to_vec();
                        out.push(0);
                        if mem.write(b, &out) {
                            return k as i32;
                        }
                    }
                }
            }
            24 => {
                if let Some(i) = idx {
                    self.widgets[i].locked = b == 0;
                }
            }
            23 => return self.net_form_op(a, b, c, mem),
            _ => {}
        }
        0
    }

    fn setup(&mut self, a: i32, b: i32, c: i32, d: i32, mem: &mut Mem) {
        match a {
            1 if c > 0 && (c as usize) < MAX_LAYOUT => {
                if let Some(t) = mem.read(b, c, MAX_LAYOUT).filter(|t| t.len() == c as usize) {
                    self.layout = t;
                }
            }
            2 if c > 0 && c as usize <= MAX_WIDGETS => {
                let want = c as usize * 16;
                if let Some(t) = mem.read(b, want as i32, want).filter(|t| t.len() == want) {
                    self.load_table(&t);
                }
            }
            3 => {
                self.mask = b & 3;
                self.system = b & 4 != 0;
            }
            4 if c >= 0 => {
                if let Some(t) = mem.read(b, c, usize::MAX) {
                    let t = &t[..t.len().min(64)];
                    let n = t.iter().position(|&x| x == 0).unwrap_or(t.len());
                    self.password = String::from_utf8_lossy(&t[..n]).into_owned();
                }
            }
            5 if c > 0 && c < 700 => {
                if let Some(t) = mem.read(b, c, 700) {
                    self.load_net_defaults(&String::from_utf8_lossy(&t), d);
                }
            }
            63 => self.complete = true,
            _ => {}
        }
    }

    /// The widget table, 16 bytes each: kind, rw, dec, reserved, lo, hi, initial (f32 LE).
    fn load_table(&mut self, t: &[u8]) {
        let mut old = core::mem::take(&mut self.widgets);
        for (i, row) in t.chunks_exact(16).enumerate() {
            let f =
                |at: usize| f32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]]);
            let mut w = Widget {
                kind: row[0],
                rw: row[1] != 0,
                dec: row[2],
                lo: f(4),
                hi: f(8),
                value: f(12),
                ..Widget::default()
            };
            if w.kind == KIND_CHART {
                let n = usize::from(row[3].clamp(10, 120));
                // The history survives a module reload of the same size.
                match old.get_mut(i).filter(|o| o.hist.len() == n) {
                    Some(o) => {
                        w.hist = core::mem::take(&mut o.hist);
                        w.hpos = o.hpos;
                        w.hfill = o.hfill;
                        w.hlast = o.hlast;
                    }
                    None => w.hist = vec![0.0; n],
                }
                let period = if w.value < 1.0 { 1 } else { w.value as u32 };
                w.hperiod = period.saturating_mul(1000);
                w.value = 0.0;
            }
            self.widgets.push(w);
        }
    }

    /// Ten lines of defaults; saved page values replace them unless the defaults changed.
    fn load_net_defaults(&mut self, text: &str, item: i32) {
        let mut init: [String; NET_FIELDS] = Default::default();
        let mut k = 0;
        let mut cur = String::new();
        for ch in text.chars() {
            if k >= NET_FIELDS {
                break;
            }
            if ch == '\n' {
                init[k] = core::mem::take(&mut cur);
                k += 1;
            } else {
                cur.push(ch);
            }
        }
        if k < NET_FIELDS {
            init[k] = cur;
        }
        let sig = fnv_fields(&init);
        self.net_default_sig = sig;
        self.net_fields = match self.net_restored.take() {
            Some(s) if s.sig == sig => s.fields,
            Some(_) => {
                // The defaults changed (another module): the saved values go.
                self.net_dirty = true;
                init
            }
            None => init,
        };
        self.net_item = usize::try_from(item).ok();
        self.net_on = true;
        self.net_show();
    }

    /// The form text the page shows; passwords are never sent back.
    fn net_show(&mut self) {
        let Some(i) = self.net_item.filter(|&i| i < self.widgets.len()) else {
            return;
        };
        let f = &self.net_fields;
        let t = alloc::format!(
            "{}\n\n{}\n{}\n{}\n{}\n{}\n\n{}\n{}",
            f[0],
            f[2],
            f[3],
            f[4],
            f[5],
            f[6],
            f[8],
            f[9]
        );
        self.widgets[i].text = clip_text(t.as_bytes());
    }

    fn net_form_op(&mut self, a: i32, b: i32, c: i32, mem: &mut Mem) -> i32 {
        match a {
            10 => i32::from(core::mem::take(&mut self.net_changed)),
            11 => i32::from(self.net_dhcp()),
            12 => self.net_mode(),
            0..=9 if b >= 0 && c > 0 => {
                let f = self.net_fields[a as usize].as_bytes();
                let k = f.len().min(c as usize - 1);
                let mut out = f[..k].to_vec();
                out.push(0);
                if mem.write(b, &out) {
                    k as i32
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

const JSON: &str = "application/json";
const PLAIN: &str = "text/plain";
const HTML: &str = "text/html; charset=utf-8";

/// Escapes a text for a JSON string the way the C++ `webEsc` does (newline, quote, backslash;
/// other control bytes are dropped).
fn json_esc(out: &mut Vec<u8>, s: &[u8]) {
    for &b in s {
        match b {
            b'\n' => out.extend_from_slice(b"\\n"),
            b'"' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b if b >= 32 => out.push(b),
            _ => {}
        }
    }
}

/// A float with `dec` digits; NaN and infinity become 0 so the JSON stays valid.
fn fmt_num(v: f32, dec: usize) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    alloc::format!("{v:.dec$}")
}

fn pw_ok(p: &str) -> bool {
    if p.is_empty() {
        return true;
    }
    if p.bytes().any(|b| !(32..=126).contains(&b)) {
        return false;
    }
    match p.len() {
        8..=63 => true,
        64 => p.bytes().all(|b| b.is_ascii_hexdigit()),
        _ => false,
    }
}

fn ssid_ok(s: &str) -> bool {
    s.len() <= 32 && s.bytes().all(|b| b >= 32)
}

impl WebState {
    /// Charts store a point of the shown value once per period. Call it often (each loop).
    pub fn tick(&mut self, now_ms: u32) {
        for w in &mut self.widgets {
            let n = w.hist.len();
            if n == 0 || (w.hfill > 0 && now_ms.wrapping_sub(w.hlast) < w.hperiod) {
                continue;
            }
            w.hlast = now_ms;
            w.hist[w.hpos] = if w.value.is_finite() { w.value } else { 0.0 };
            w.hpos = (w.hpos + 1) % n;
            w.hfill = (w.hfill + 1).min(n);
        }
    }

    fn allowed(&self, req: &Request) -> bool {
        self.password.is_empty() || req.arg("k") == Some(self.password.as_str())
    }

    /// Answers a request of the page. The page itself is public; everything it reads or writes
    /// needs the password (argument `k`).
    pub fn handle(&mut self, req: &Request, sys: &SystemInfo) -> Response {
        if req.path == "/" {
            return self.page_response();
        }
        let known = match req.path.as_str() {
            "/ui/layout" | "/ui/state" | "/ui/set" | "/ui/hist" => true,
            "/ui/info" => self.system,
            "/ui/net" | "/ui/scan" => self.net_on,
            _ => false,
        };
        if !known {
            return Response::new(404, PLAIN, "not found");
        }
        if !self.allowed(req) {
            return Response::new(403, PLAIN, "auth");
        }
        match req.path.as_str() {
            "/ui/layout" => {
                if self.layout.is_empty() {
                    Response::new(200, JSON, "{\"i\":[]}")
                } else {
                    Response::new(200, JSON, self.layout.clone())
                }
            }
            "/ui/state" => self.state_json(),
            "/ui/set" => self.set(req),
            "/ui/hist" => self.hist_json(req),
            "/ui/net" => self.net_set(req),
            "/ui/scan" => {
                self.scan_wanted = true;
                Response::new(200, PLAIN, sys.scan.clone())
            }
            _ => Response::new(200, JSON, Self::info_json(sys)),
        }
    }

    fn page_response(&self) -> Response {
        if self.layout.len() < 12 {
            return Response::new(
                200,
                HTML,
                "<!DOCTYPE html><meta charset=\"utf-8\"><title>CRAB</title><p>Страница не загружена",
            );
        }
        Response::new(200, HTML, self.page)
    }

    fn state_json(&self) -> Response {
        let mut o = vec![b'['];
        for (i, w) in self.widgets.iter().enumerate() {
            if i > 0 {
                o.push(b',');
            }
            if w.kind == KIND_NUMBER || w.kind == KIND_CHART {
                o.extend_from_slice(fmt_num(w.value, usize::from(w.dec)).as_bytes());
            } else {
                o.push(b'"');
                // A secret field is never sent back to the page.
                if w.kind != KIND_SECRET {
                    json_esc(&mut o, &w.text);
                }
                o.push(b'"');
            }
        }
        o.push(b']');
        Response::new(200, JSON, o)
    }

    fn set(&mut self, req: &Request) -> Response {
        if let (Some(i), Some(v)) = (req.arg("i"), req.arg("v")) {
            let i = to_int(i);
            if let Some(w) = usize::try_from(i)
                .ok()
                .and_then(|i| self.widgets.get_mut(i))
                .filter(|w| w.rw && !w.locked)
            {
                if w.kind >= KIND_TEXT_INPUT && w.kind != KIND_CHART {
                    let t: Vec<u8> = v
                        .bytes()
                        .map(|b| if b == b'\n' || b == b'\r' { b' ' } else { b })
                        .collect();
                    w.text = clip_text(&t);
                } else if w.kind == KIND_NUMBER {
                    let mut f = to_float(v);
                    if w.hi > w.lo {
                        f = f.clamp(w.lo, w.hi);
                    }
                    w.value = f;
                }
            }
        }
        Response::new(200, PLAIN, "ok")
    }

    fn hist_json(&self, req: &Request) -> Response {
        let mut o = vec![b'['];
        let i = to_int(req.arg("i").unwrap_or(""));
        if let Some(w) = usize::try_from(i)
            .ok()
            .and_then(|i| self.widgets.get(i))
            .filter(|w| !w.hist.is_empty())
        {
            let n = w.hist.len();
            let start = (w.hpos + n - w.hfill) % n;
            for k in 0..w.hfill {
                if k > 0 {
                    o.push(b',');
                }
                o.extend_from_slice(fmt_num(w.hist[(start + k) % n], 2).as_bytes());
            }
        }
        o.push(b']');
        Response::new(200, JSON, o)
    }

    fn net_set(&mut self, req: &Request) -> Response {
        let mut nf: [String; NET_FIELDS] = Default::default();
        for (k, key) in NET_KEYS.iter().enumerate() {
            nf[k] = req.arg(key).unwrap_or("").replace(['\n', '\r'], " ");
        }
        // An empty password keeps the saved one.
        for k in [1, 7] {
            if nf[k].is_empty() {
                nf[k] = self.net_fields[k].clone();
            }
        }
        if !self.net_on || !pw_ok(&nf[1]) || !pw_ok(&nf[7]) || !ssid_ok(&nf[0]) || !ssid_ok(&nf[6])
        {
            return Response::new(400, PLAIN, "bad");
        }
        for (k, new) in nf.iter_mut().enumerate() {
            if *new != self.net_fields[k] {
                self.net_fields[k] = core::mem::take(new);
                self.net_changed = true;
                self.net_dirty = true;
            }
        }
        self.net_show();
        Response::new(200, PLAIN, "ok")
    }

    fn info_json(s: &SystemInfo) -> String {
        let mut o = String::from("{\"ip\":\"");
        let esc = |t: &str| {
            let mut v = Vec::new();
            json_esc(&mut v, t.as_bytes());
            String::from_utf8_lossy(&v).into_owned()
        };
        o += &alloc::format!(
            "{}\",\"mac\":\"{}\",\"rssi\":{},\"up\":{},\"chip\":\"{}\",\"cpu\":{},\"heap\":{},\"heapAll\":{},\"flash\":{},\"sketch\":{},\"free\":{},\"build\":\"{}\"}}",
            esc(&s.ip),
            esc(&s.mac),
            s.rssi,
            s.uptime_s,
            esc(&s.chip),
            s.cpu_mhz,
            s.heap_free,
            s.heap_total,
            s.flash_size,
            s.app_size,
            s.app_free,
            esc(&s.build)
        );
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: u8, rw: u8, dec: u8, points: u8, lo: f32, hi: f32, init: f32) -> Vec<u8> {
        let mut r = vec![kind, rw, dec, points];
        for x in [lo, hi, init] {
            r.extend_from_slice(&x.to_le_bytes());
        }
        r
    }

    /// Widgets: 0 number rw 0..10, 1 text shown, 2 text input, 3 secret, 4 chart (10 points, 1 s).
    fn board(password: &str, net: bool) -> WebState {
        let mut ram = vec![0u8; 2048];
        let table = [
            row(0, 1, 1, 0, 0.0, 10.0, 2.5),
            row(1, 0, 0, 0, 0.0, 0.0, 0.0),
            row(2, 1, 0, 0, 0.0, 0.0, 0.0),
            row(3, 1, 0, 0, 0.0, 0.0, 0.0),
            row(4, 0, 1, 10, 0.0, 0.0, 1.0),
            row(1, 1, 0, 0, 0.0, 0.0, 0.0),
        ]
        .concat();
        ram[0..table.len()].copy_from_slice(&table);
        let layout = b"{\"t\":\"x\",\"p\":\"a\",\"y\":1,\"i\":[]}";
        ram[200..200 + layout.len()].copy_from_slice(layout);
        ram[300..300 + password.len()].copy_from_slice(password.as_bytes());
        let defaults = b"home\nsecret12\n1\n\n\n\n\n\n0\n30";
        ram[400..400 + defaults.len()].copy_from_slice(defaults);
        let mut w = WebState::new("<html>PAGE</html>");
        let mut mem = Mem::new(Some(&mut ram));
        w.net_op(17, 2, 0, 6, 0, &mut mem, 0);
        w.net_op(17, 1, 200, layout.len() as i32, 0, &mut mem, 0);
        w.net_op(17, 3, 7, 0, 0, &mut mem, 0);
        w.net_op(17, 4, 300, password.len() as i32, 0, &mut mem, 0);
        if net {
            w.net_op(17, 5, 400, defaults.len() as i32, 5, &mut mem, 0);
        }
        w.net_op(17, 63, 0, 0, 0, &mut mem, 0);
        w
    }

    fn get(w: &mut WebState, path: &str, q: &[(&str, &str)]) -> Response {
        w.handle(&Request::get(path, q), &SystemInfo::default())
    }

    #[test]
    fn page_is_public_and_state_needs_password() {
        let mut w = board("pw", false);
        assert!(w.complete());
        assert_eq!(w.interfaces(), 3);
        assert!(w.system_page());
        assert_eq!(get(&mut w, "/", &[]).text(), "<html>PAGE</html>");
        let r = get(&mut w, "/ui/state", &[]);
        assert_eq!((r.status, r.text().as_str()), (403, "auth"));
        assert_eq!(get(&mut w, "/ui/state", &[("k", "bad")]).status, 403);
        assert_eq!(
            get(&mut w, "/ui/set", &[("i", "0"), ("v", "1")]).status,
            403
        );
        assert_eq!(get(&mut w, "/ui/layout", &[("k", "pw")]).status, 200);
        let r = get(&mut w, "/ui/state", &[("k", "pw")]);
        assert_eq!(r.text(), "[2.5,\"\",\"\",\"\",0.0,\"\"]");
        assert_eq!(get(&mut w, "/nope", &[]).status, 404);
        assert_eq!(get(&mut w, "/ui/net", &[("k", "pw")]).status, 404);
    }

    #[test]
    fn no_password_and_missing_layout() {
        let mut w = WebState::new("PAGE");
        assert!(get(&mut w, "/", &[])
            .text()
            .contains("Страница не загружена"));
        assert_eq!(get(&mut w, "/ui/layout", &[]).text(), "{\"i\":[]}");
        assert_eq!(get(&mut w, "/ui/state", &[]).text(), "[]");
    }

    #[test]
    fn show_and_read_values() {
        let mut w = board("", false);
        let mut ram = vec![0u8; 64];
        ram[0..5].copy_from_slice(b"hello");
        let mut mem = Mem::new(Some(&mut ram));
        w.net_op(19, 0, 3.25f32.to_bits() as i32, 0, 0, &mut mem, 0);
        w.net_op(20, 1, 0, 5, 0, &mut mem, 0);
        assert_eq!(
            get(&mut w, "/ui/state", &[]).text(),
            "[3.2,\"hello\",\"\",\"\",0.0,\"\"]"
        );
        // page writes: number is clamped, text has no line breaks, locked widgets refuse
        get(&mut w, "/ui/set", &[("i", "0"), ("v", "99")]);
        get(&mut w, "/ui/set", &[("i", "2"), ("v", "a\nb")]);
        get(&mut w, "/ui/set", &[("i", "3"), ("v", "topsecret")]);
        get(&mut w, "/ui/set", &[("i", "1"), ("v", "no")]);
        assert_eq!(
            f32::from_bits(w.net_op(21, 0, 0, 0, 0, &mut mem, 0) as u32),
            10.0
        );
        assert_eq!(w.net_op(22, 2, 10, 32, 0, &mut mem, 0), 3);
        assert_eq!(&mem.bytes().unwrap()[10..14], b"a b\0");
        assert_eq!(w.net_op(22, 3, 20, 5, 0, &mut mem, 0), 4);
        assert_eq!(&mem.bytes().unwrap()[20..25], b"tops\0");
        assert_eq!(w.net_op(22, 1, 30, 8, 0, &mut mem, 0), 5);
        let s = get(&mut w, "/ui/state", &[]).text();
        assert!(s.contains("\"a b\"") && s.contains("\"\",0.0"));
        w.net_op(24, 2, 0, 0, 0, &mut mem, 0);
        get(&mut w, "/ui/set", &[("i", "2"), ("v", "x")]);
        assert_eq!(w.net_op(22, 2, 10, 32, 0, &mut mem, 0), 3);
    }

    #[test]
    fn escapes_text() {
        let mut w = board("", false);
        let mut ram = *b"a\"b\\c\nd";
        let mut mem = Mem::new(Some(&mut ram));
        w.net_op(20, 1, 0, 7, 0, &mut mem, 0);
        assert!(get(&mut w, "/ui/state", &[])
            .text()
            .contains("\"a\\\"b\\\\c\\nd\""));
    }

    #[test]
    fn chart_sampling_and_history() {
        let mut w = board("", false);
        let mut ram = vec![0u8; 8];
        let mut mem = Mem::new(Some(&mut ram));
        let hist = |w: &mut WebState| get(w, "/ui/hist", &[("i", "4")]).text();
        assert_eq!(hist(&mut w), "[]");
        for s in 0..13u32 {
            w.net_op(19, 4, (s as f32).to_bits() as i32, 0, 0, &mut mem, 0);
            w.tick(s * 1000);
            w.tick(s * 1000 + 400); // the same period: no second point
            if s == 2 {
                assert_eq!(hist(&mut w), "[0.00,1.00,2.00]");
            }
        }
        // 10 points kept, oldest first
        assert_eq!(
            hist(&mut w),
            "[3.00,4.00,5.00,6.00,7.00,8.00,9.00,10.00,11.00,12.00]"
        );
        assert_eq!(get(&mut w, "/ui/hist", &[("i", "0")]).text(), "[]");
        assert_eq!(get(&mut w, "/ui/hist", &[]).text(), "[]");
        // a module reload of the same size keeps the history
        let table = [[0u8; 64].to_vec(), row(4, 0, 1, 10, 0.0, 0.0, 1.0)].concat();
        let mut ram2 = vec![0u8; 80];
        ram2.copy_from_slice(&table);
        let mut m2 = Mem::new(Some(&mut ram2));
        w.net_op(17, 2, 0, 5, 0, &mut m2, 0);
        assert!(hist(&mut w).ends_with("12.00]"));
    }

    #[test]
    fn layout_persistence() {
        let mut w = board("", false);
        let (bytes, hash) = w.layout_for_save().unwrap();
        let (bytes, hash) = (bytes.to_vec(), hash);
        let mut fresh = WebState::new("P");
        assert!(!fresh.restore_layout(&bytes, hash ^ 1));
        assert!(fresh.restore_layout(&bytes, hash));
        assert!(!fresh.restore_layout(&bytes, hash));
        assert_eq!(get(&mut fresh, "/ui/layout", &[]).body, bytes);
        assert_eq!(get(&mut w, "/ui/layout", &[]).body, bytes);
    }

    #[test]
    fn system_info() {
        let mut w = board("", false);
        let sys = SystemInfo {
            ip: "1.2.3.4".into(),
            chip: "ESP32".into(),
            uptime_s: 5,
            ..SystemInfo::default()
        };
        let r = w.handle(&Request::get("/ui/info", &[]), &sys).text();
        assert!(r.starts_with(
            "{\"ip\":\"1.2.3.4\",\"mac\":\"\",\"rssi\":0,\"up\":5,\"chip\":\"ESP32\""
        ));
        assert!(r.contains("\"heapAll\":0") && r.ends_with("\"build\":\"\"}"));
    }

    #[test]
    fn network_form_round_trip() {
        let mut w = board("", true);
        assert!(w.net_form());
        // text the page sees: no passwords
        let s = get(&mut w, "/ui/state", &[]).text();
        assert!(s.contains("\"home\\n\\n1\\n\\n\\n\\n\\n\\n0\\n30\""), "{s}");
        let ok = [
            ("s", "net2"),
            ("p", ""),
            ("d", "0"),
            ("i", "10.0.0.5"),
            ("g", "10.0.0.1"),
            ("m", "255.0.0.0"),
            ("a", "ap"),
            ("b", "12345678"),
            ("o", "3"),
            ("w", "20"),
        ];
        assert_eq!(get(&mut w, "/ui/net", &[("p", "short")]).status, 400);
        assert_eq!(
            get(&mut w, "/ui/net", &[("s", &"x".repeat(33))]).status,
            400
        );
        // empty password keeps the saved one
        let sig = w.net_signature();
        let r = get(&mut w, "/ui/net", &ok);
        assert_eq!(r.text(), "ok");
        assert_ne!(sig, w.net_signature());
        assert_eq!(w.net_fields()[1], "secret12");
        assert_eq!(w.net_fields()[7], "12345678");
        assert!(!w.net_dhcp());
        assert_eq!(w.net_mode(), 3);
        // module side (op 23)
        let mut ram = vec![0u8; 64];
        let mut mem = Mem::new(Some(&mut ram));
        assert_eq!(w.net_op(23, 0, 0, 16, 0, &mut mem, 0), 4);
        assert_eq!(&mem.bytes().unwrap()[0..5], b"net2\0");
        assert_eq!(w.net_op(23, 3, 8, 4, 0, &mut mem, 0), 3);
        assert_eq!(&mem.bytes().unwrap()[8..12], b"10.\0");
        assert_eq!(
            (
                w.net_op(23, 11, 0, 0, 0, &mut mem, 0),
                w.net_op(23, 12, 0, 0, 0, &mut mem, 0)
            ),
            (0, 3)
        );
        assert_eq!(w.net_op(23, 10, 0, 0, 0, &mut mem, 0), 1);
        assert_eq!(w.net_op(23, 10, 0, 0, 0, &mut mem, 0), 0);
        // an unchanged save does not flag again
        get(&mut w, "/ui/net", &ok);
        assert_eq!(w.net_op(23, 10, 0, 0, 0, &mut mem, 0), 0);
        // persistence: same defaults keep the saved values, other defaults drop them
        let saved = w.take_net_save().unwrap();
        assert!(w.take_net_save().is_none());
        let mut again = board("", false);
        let n = 25;
        again.restore_net(saved.clone());
        let mut ram = *b"home\nsecret12\n1\n\n\n\n\n\n0\n30";
        let mut mem = Mem::new(Some(&mut ram));
        again.net_op(2, 0, 0, 0, 0, &mut mem, 0);
        again.net_op(17, 5, 0, n, 5, &mut mem, 0);
        assert_eq!(again.net_fields(), &saved.fields);
        let mut other = board("", false);
        other.restore_net(saved);
        let mut ram = *b"other\nsecret12\n1\n\n\n\n\n\n0\n30";
        let mut mem = Mem::new(Some(&mut ram));
        other.net_op(17, 5, 0, 26, 5, &mut mem, 0);
        assert_eq!(other.net_fields()[0], "other");
        assert!(other.take_net_save().is_some());
    }

    #[test]
    fn scan_route() {
        let mut w = board("", true);
        let sys = SystemInfo {
            scan: "?".into(),
            ..SystemInfo::default()
        };
        assert_eq!(w.handle(&Request::get("/ui/scan", &[]), &sys).text(), "?");
        assert!(w.scan_wanted());
        assert!(!w.scan_wanted());
    }

    #[test]
    fn setup_table_and_show_number() {
        let mut ram = vec![0u8; 256];
        let table = [
            row(0, 1, 1, 0, 0.0, 10.0, 2.5),
            row(1, 0, 0, 0, 0.0, 0.0, 0.0),
        ]
        .concat();
        ram[64..64 + 32].copy_from_slice(&table);
        let mut w = WebState::new("<html>");
        let mut mem = Mem::new(Some(&mut ram));
        w.net_op(17, 2, 64, 2, 0, &mut mem, 0);
        assert_eq!(
            f32::from_bits(w.net_op(21, 0, 0, 0, 0, &mut mem, 0) as u32),
            2.5
        );
        w.net_op(19, 0, 7.0f32.to_bits() as i32, 0, 0, &mut mem, 0);
        assert_eq!(
            f32::from_bits(w.net_op(21, 0, 0, 0, 0, &mut mem, 0) as u32),
            7.0
        );
    }

    #[test]
    fn uri_query_is_decoded() {
        let r = Request::from_uri("GET", "/ui/set?i=3&v=a%20b+c%D0%96&k=", Vec::new());
        assert_eq!(r.path, "/ui/set");
        assert_eq!(r.arg("i"), Some("3"));
        assert_eq!(r.arg("v"), Some("a b cЖ"));
        assert_eq!(r.arg("k"), Some(""));
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(Request::from_uri("GET", "/", Vec::new()).query.len(), 0);
    }
}
