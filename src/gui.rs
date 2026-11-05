use crate::engine::{
    fmt_duration, lock, save_history, save_settings, start_engine, App, History, Settings,
    VERSION,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::process::Command;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

type Handle = *mut c_void;
type Gp = *mut c_void;

#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: unsafe extern "system" fn(Handle, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
struct Msg {
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    private: u32,
}

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    hdc: Handle,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct TrackMouse {
    size: u32,
    flags: u32,
    hwnd: Handle,
    hover_time: u32,
}

#[repr(C)]
#[derive(Default)]
struct LocalTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millis: u16,
}

#[repr(C)]
struct GdipStartupInput {
    version: u32,
    callback: *const c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[repr(C)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: Handle,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn ShowWindow(hwnd: Handle, cmd: i32) -> i32;
    fn UpdateWindow(hwnd: Handle) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
    fn PostMessageW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn SendMessageW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn DestroyWindow(hwnd: Handle) -> i32;
    fn BeginPaint(hwnd: Handle, ps: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: Handle, ps: *const PaintStruct) -> i32;
    fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
    fn SetTimer(hwnd: Handle, id: usize, ms: u32, func: *const c_void) -> usize;
    fn KillTimer(hwnd: Handle, id: usize) -> i32;
    fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
    fn SetCapture(hwnd: Handle) -> Handle;
    fn ReleaseCapture() -> i32;
    fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
    fn MessageBoxW(hwnd: Handle, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn SetCursor(cursor: Handle) -> Handle;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn GetDC(hwnd: Handle) -> Handle;
    fn ReleaseDC(hwnd: Handle, hdc: Handle) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: Handle) -> Handle;
    fn CreateCompatibleBitmap(hdc: Handle, w: i32, h: i32) -> Handle;
    fn SelectObject(hdc: Handle, object: Handle) -> Handle;
    fn BitBlt(
        dst: Handle,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: Handle,
        sx: i32,
        sy: i32,
        rop: u32,
    ) -> i32;
    fn DeleteObject(object: Handle) -> i32;
    fn DeleteDC(hdc: Handle) -> i32;
    fn GetDeviceCaps(hdc: Handle, index: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetLocalTime(time: *mut LocalTime);
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: Handle, attr: u32, value: *const c_void, size: u32) -> i32;
}

#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(token: *mut usize, input: *const GdipStartupInput, output: *mut c_void) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(hdc: Handle, graphics: *mut Gp) -> i32;
    fn GdipDeleteGraphics(graphics: Gp) -> i32;
    fn GdipSetSmoothingMode(graphics: Gp, mode: i32) -> i32;
    fn GdipSetTextRenderingHint(graphics: Gp, hint: i32) -> i32;
    fn GdipSetPixelOffsetMode(graphics: Gp, mode: i32) -> i32;
    fn GdipScaleWorldTransform(graphics: Gp, sx: f32, sy: f32, order: i32) -> i32;
    fn GdipGraphicsClear(graphics: Gp, color: u32) -> i32;
    fn GdipSetClipRect(graphics: Gp, x: f32, y: f32, w: f32, h: f32, mode: i32) -> i32;
    fn GdipResetClip(graphics: Gp) -> i32;

    fn GdipCreateSolidFill(color: u32, brush: *mut Gp) -> i32;
    fn GdipCreateLineBrush(
        p1: *const PointF,
        p2: *const PointF,
        c1: u32,
        c2: u32,
        wrap: i32,
        brush: *mut Gp,
    ) -> i32;
    fn GdipDeleteBrush(brush: Gp) -> i32;
    fn GdipCreatePen1(color: u32, width: f32, unit: i32, pen: *mut Gp) -> i32;
    fn GdipSetPenStartCap(pen: Gp, cap: i32) -> i32;
    fn GdipSetPenEndCap(pen: Gp, cap: i32) -> i32;
    fn GdipDeletePen(pen: Gp) -> i32;

    fn GdipCreatePath(fill_mode: i32, path: *mut Gp) -> i32;
    fn GdipDeletePath(path: Gp) -> i32;
    fn GdipAddPathArc(path: Gp, x: f32, y: f32, w: f32, h: f32, start: f32, sweep: f32) -> i32;
    fn GdipAddPathRectangle(path: Gp, x: f32, y: f32, w: f32, h: f32) -> i32;
    fn GdipAddPathLine2(path: Gp, points: *const PointF, count: i32) -> i32;
    fn GdipClosePathFigure(path: Gp) -> i32;
    fn GdipFillPath(graphics: Gp, brush: Gp, path: Gp) -> i32;
    fn GdipDrawPath(graphics: Gp, pen: Gp, path: Gp) -> i32;
    fn GdipFillEllipse(graphics: Gp, brush: Gp, x: f32, y: f32, w: f32, h: f32) -> i32;
    fn GdipFillRectangle(graphics: Gp, brush: Gp, x: f32, y: f32, w: f32, h: f32) -> i32;
    fn GdipDrawArc(
        graphics: Gp,
        pen: Gp,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        start: f32,
        sweep: f32,
    ) -> i32;
    fn GdipDrawLine(graphics: Gp, pen: Gp, x1: f32, y1: f32, x2: f32, y2: f32) -> i32;

    fn GdipCreateFontFamilyFromName(name: *const u16, collection: Gp, family: *mut Gp) -> i32;
    fn GdipDeleteFontFamily(family: Gp) -> i32;
    fn GdipCreateFont(family: Gp, size: f32, style: i32, unit: i32, font: *mut Gp) -> i32;
    fn GdipDeleteFont(font: Gp) -> i32;
    fn GdipCreateStringFormat(flags: i32, language: u16, format: *mut Gp) -> i32;
    fn GdipDeleteStringFormat(format: Gp) -> i32;
    fn GdipSetStringFormatAlign(format: Gp, align: i32) -> i32;
    fn GdipSetStringFormatLineAlign(format: Gp, align: i32) -> i32;
    fn GdipSetStringFormatTrimming(format: Gp, trimming: i32) -> i32;
    fn GdipDrawString(
        graphics: Gp,
        text: *const u16,
        length: i32,
        font: Gp,
        rect: *const RectF,
        format: Gp,
        brush: Gp,
    ) -> i32;
}

const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const HTCLIENT: isize = 1;
const HTCAPTION: usize = 2;

const WS_POPUP: u32 = 0x8000_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;

const MB_YESNO: u32 = 0x04;
const MB_ICONWARNING: u32 = 0x30;
const MB_ICONERROR: u32 = 0x10;
const IDYES: i32 = 6;

const TIMER_ID: usize = 1;

const W: f32 = 980.0;
const H: f32 = 660.0;

const BG: u32 = 0xFF0E1016;
const CARD: u32 = 0xFF171A23;
const CARD_BORDER: u32 = 0xFF252A37;
const SURFACE: u32 = 0xFF11131A;
const BUTTON: u32 = 0xFF222634;
const TEXT: u32 = 0xFFEDEFF5;
const MUTED: u32 = 0xFF8A90A2;
const ACCENT_A: u32 = 0xFF7C5CFF;
const ACCENT_B: u32 = 0xFF00C2FF;
const GREEN: u32 = 0xFF2ED47A;
const RED: u32 = 0xFFFF5C7A;
const RED_B: u32 = 0xFFFF8A4C;
const YELLOW: u32 = 0xFFFFC857;

const SL_X: f32 = 44.0;
const SL_W: f32 = 380.0;
const SL_Y0: f32 = 176.0;
const SL_STEP: f32 = 68.0;

const LOG_X: f32 = 480.0;
const LOG_Y: f32 = 358.0;
const LOG_W: f32 = 460.0;
const LOG_H: f32 = 264.0;
const LOG_LINE: f32 = 19.0;
const MAX_UI_LOG: usize = 500;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn with_alpha(color: u32, alpha: f32) -> u32 {
    let a = ((color >> 24) as f32 * alpha.clamp(0.0, 1.0)).round() as u32;
    (a.min(255) << 24) | (color & 0x00FF_FFFF)
}
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xFF) as f32;
        let y = ((b >> shift) & 0xFF) as f32;
        ((x + (y - x) * t).round() as u32).min(255) << shift
    };
    channel(24) | channel(16) | channel(8) | channel(0)
}

pub fn error_box(text: &str) {
    let text = wide(text);
    let caption = wide("QDCord");
    unsafe {
        MessageBoxW(ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR);
    }
}

fn confirm(hwnd: Handle, text: &str) -> bool {
    let text = wide(text);
    let caption = wide("QDCord");
    unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONWARNING) == IDYES
    }
}

#[derive(Clone, Copy, Default)]
struct Anim {
    value: f32,
    target: f32,
}

impl Anim {
    fn new(value: f32) -> Self {
        Self { value, target: value }
    }

    fn step(&mut self, dt: f32, speed: f32) -> bool {
        let diff = self.target - self.value;
        if diff.abs() < 0.001 {
            self.value = self.target;
            return false;
        }
        self.value += diff * (1.0 - (-dt * speed).exp());
        true
    }
}

struct Painter {
    g: Gp,
    alpha: f32,
}

impl Painter {
    fn col(&self, color: u32) -> u32 {
        with_alpha(color, self.alpha)
    }

    fn rr_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Gp {
        let mut path: Gp = ptr::null_mut();
        unsafe {
            GdipCreatePath(0, &mut path);
            if path.is_null() {
                return path;
            }
            let r = r.min(w / 2.0).min(h / 2.0);
            if r < 0.5 {
                GdipAddPathRectangle(path, x, y, w, h);
            } else {
                let d = r * 2.0;
                GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
                GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
                GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
                GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
                GdipClosePathFigure(path);
            }
        }
        path
    }

    fn fill_with(&self, brush: Gp, x: f32, y: f32, w: f32, h: f32, r: f32) {
        if brush.is_null() {
            return;
        }
        let path = Self::rr_path(x, y, w, h, r);
        unsafe {
            if !path.is_null() {
                GdipFillPath(self.g, brush, path);
                GdipDeletePath(path);
            }
            GdipDeleteBrush(brush);
        }
    }

    fn fill_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mut brush: Gp = ptr::null_mut();
        unsafe {
            GdipCreateSolidFill(self.col(color), &mut brush);
        }
        self.fill_with(brush, x, y, w, h, r);
    }

    fn fill_rr_grad(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c1: u32, c2: u32) {
        if w < 1.0 || h <= 0.0 {
            return;
        }
        let p1 = PointF { x: x - 0.5, y };
        let p2 = PointF { x: x + w + 0.5, y };
        let mut brush: Gp = ptr::null_mut();
        unsafe {
            GdipCreateLineBrush(&p1, &p2, self.col(c1), self.col(c2), 3, &mut brush);
        }
        self.fill_with(brush, x, y, w, h, r);
    }

    //вертикальный градиент прямоугольника
    fn fill_rect_vgrad(&self, x: f32, y: f32, w: f32, h: f32, c1: u32, c2: u32) {
        if h < 1.0 || w <= 0.0 {
            return;
        }
        let p1 = PointF { x, y: y - 0.5 };
        let p2 = PointF { x, y: y + h + 0.5 };
        let mut brush: Gp = ptr::null_mut();
        unsafe {
            GdipCreateLineBrush(&p1, &p2, self.col(c1), self.col(c2), 3, &mut brush);
            if !brush.is_null() {
                GdipFillRectangle(self.g, brush, x, y, w, h);
                GdipDeleteBrush(brush);
            }
        }
    }

    fn pen(&self, color: u32, width: f32) -> Gp {
        let mut pen: Gp = ptr::null_mut();
        unsafe {
            GdipCreatePen1(self.col(color), width, 0, &mut pen);
            if !pen.is_null() {
                GdipSetPenStartCap(pen, 2);
                GdipSetPenEndCap(pen, 2);
            }
        }
        pen
    }
    fn stroke_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32, width: f32) {
        let pen = self.pen(color, width);
        if pen.is_null() {
            return;
        }
        let path = Self::rr_path(x, y, w, h, r);
        unsafe {
            if !path.is_null() {
                GdipDrawPath(self.g, pen, path);
                GdipDeletePath(path);
            }
            GdipDeletePen(pen);
        }
    }

    fn circle(&self, cx: f32, cy: f32, r: f32, color: u32) {
        if r <= 0.0 {
            return;
        }
        let mut brush: Gp = ptr::null_mut();
        unsafe {
            GdipCreateSolidFill(self.col(color), &mut brush);
            if !brush.is_null() {
                GdipFillEllipse(self.g, brush, cx - r, cy - r, r * 2.0, r * 2.0);
                GdipDeleteBrush(brush);
            }
        }
    }

    fn arc(&self, cx: f32, cy: f32, r: f32, start: f32, sweep: f32, color: u32, width: f32) {
        let pen = self.pen(color, width);
        if pen.is_null() {
            return;
        }
        unsafe {
            GdipDrawArc(self.g, pen, cx - r, cy - r, r * 2.0, r * 2.0, start, sweep);
            GdipDeletePen(pen);
        }
    }

    fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) {
        let pen = self.pen(color, width);
        if pen.is_null() {
            return;
        }
        unsafe {
            GdipDrawLine(self.g, pen, x1, y1, x2, y2);
            GdipDeletePen(pen);
        }
    }

    fn polygon(&self, points: &[PointF], color: u32) {
        let mut brush: Gp = ptr::null_mut();
        let mut path: Gp = ptr::null_mut();
        unsafe {
            GdipCreateSolidFill(self.col(color), &mut brush);
            GdipCreatePath(0, &mut path);
            if !brush.is_null() && !path.is_null() {
                GdipAddPathLine2(path, points.as_ptr(), points.len() as i32);
                GdipClosePathFigure(path);
                GdipFillPath(self.g, brush, path);
            }
            if !path.is_null() {
                GdipDeletePath(path);
            }
            if !brush.is_null() {
                GdipDeleteBrush(brush);
            }
        }
    }

    fn text(&self, text: &str, font: Gp, format: Gp, x: f32, y: f32, w: f32, h: f32, color: u32) {
        if font.is_null() || text.is_empty() {
            return;
        }
        let text = wide(text);
        let rect = RectF { x, y, w, h };
        let mut brush: Gp = ptr::null_mut();
        unsafe {
            GdipCreateSolidFill(self.col(color), &mut brush);
            if !brush.is_null() {
                GdipDrawString(self.g, text.as_ptr(), -1, font, &rect, format, brush);
                GdipDeleteBrush(brush);
            }
        }
    }

    fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
        unsafe {
            GdipSetClipRect(self.g, x, y, w, h, 0);
        }
    }


    fn unclip(&self) {
        unsafe {
            GdipResetClip(self.g);
        }
    }
}


struct Fonts {
    families: Vec<Gp>,
    title: Gp,
    heading: Gp,
    body: Gp,
    body_bold: Gp,
    small: Gp,
    big: Gp,
    mono: Gp,
    near: Gp,
    center: Gp,
    far: Gp,
    wrap: Gp,
}

impl Fonts {
    fn new() -> Self {
        let mut families = Vec::new();

        let mut family = |names: &[&str]| -> Gp {
            for name in names {
                let name = wide(name);
                let mut family: Gp = ptr::null_mut();
                let status = unsafe {
                    GdipCreateFontFamilyFromName(name.as_ptr(), ptr::null_mut(), &mut family)
                };
                if status == 0 && !family.is_null() {
                    families.push(family);
                    return family;
                }
            }
            ptr::null_mut()
        };

        let ui = family(&["Segoe UI", "Tahoma", "Arial"]);
        let semi = family(&["Segoe UI Semibold", "Segoe UI", "Tahoma", "Arial"]);
        let mono = family(&["Cascadia Mono", "Consolas", "Courier New"]);

        let font = |family: Gp, size: f32, style: i32| -> Gp {
            let mut font: Gp = ptr::null_mut();
            if !family.is_null() {
                unsafe {
                    // 2 = UnitPixel
                    GdipCreateFont(family, size, style, 2, &mut font);
                }
            }
            font
        };

        let format = |align: i32, wrap: bool| -> Gp {
            let mut format: Gp = ptr::null_mut();
            unsafe {
                GdipCreateStringFormat(if wrap { 0 } else { 0x1000 }, 0, &mut format);
                if !format.is_null() {
                    GdipSetStringFormatAlign(format, align);
                    GdipSetStringFormatLineAlign(format, if wrap { 0 } else { 1 });
                    GdipSetStringFormatTrimming(format, 3);
                }
            }
            format
        };

        Self {
            title: font(semi, 18.0, 1),
            heading: font(semi, 15.0, 1),
            body: font(ui, 13.5, 0),
            body_bold: font(semi, 13.5, 1),
            small: font(ui, 11.5, 0),
            big: font(semi, 26.0, 1),
            mono: font(mono, 12.0, 0),
            near: format(0, false),
            center: format(1, false),
            far: format(2, false),
            wrap: format(0, true),
            families,
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for font in [
                self.title,
                self.heading,
                self.body,
                self.body_bold,
                self.small,
                self.big,
                self.mono,
            ] {
                if !font.is_null() {
                    GdipDeleteFont(font);
                }
            }
            for format in [self.near, self.center, self.far, self.wrap] {
                if !format.is_null() {
                    GdipDeleteStringFormat(format);
                }
            }
            for family in self.families.drain(..) {
                GdipDeleteFontFamily(family);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Title,
    Min,
    Close,
    Tab(u8),
    Slider(usize),
    Toggle,
    Start,
    Clear,
    Folder,
    Github,
    Telegram,
    Log,
}

impl Hit {
    fn hover_index(self) -> Option<usize> {
        match self {
            Hit::Min => Some(0),
            Hit::Close => Some(1),
            Hit::Tab(t) => Some(if t < 2 { 2 + t as usize } else { 12 }),
            Hit::Slider(i) if i < 4 => Some(4 + i),
            Hit::Toggle => Some(8),
            Hit::Start => Some(9),
            Hit::Clear => Some(10),
            Hit::Folder => Some(11),
            Hit::Github => Some(13),
            Hit::Telegram => Some(14),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Field {
    VarietyParallel,
    VarietyInterval,
    Hold,
    Limit,
    HoursParallel,
    HoursInterval,
    Duration,
}

#[derive(Clone, Copy)]
struct SliderDef {
    label: &'static str,
    field: Field,
    min: u64,
    max: u64,
    step: u64,
}

fn slider_defs(hours: bool) -> Vec<SliderDef> {
    let def = |label, field, min, max, step| SliderDef { label, field, min, max, step };
    if hours {
        vec![
            def("Процессов одновременно", Field::HoursParallel, 8, 256, 1),
            def("Пауза между запусками", Field::HoursInterval, 100, 10_000, 100),
            def("Длительность", Field::Duration, 0, 1440, 5),
        ]
    } else {
        vec![
            def("Игр в одной пачке", Field::VarietyParallel, 1, 64, 1),
            def("Пауза между запусками", Field::VarietyInterval, 100, 10_000, 100),
            def("Держать пачку", Field::Hold, 5, 3600, 5),
            def("Лимит игр за запуск", Field::Limit, 0, 2000, 10),
        ]
    }
}

fn get_field(settings: &Settings, field: Field) -> u64 {
    match field {
        Field::VarietyParallel => settings.variety_parallel as u64,
        Field::VarietyInterval => settings.variety_interval_ms,
        Field::Hold => settings.hold_seconds,
        Field::Limit => settings.limit as u64,
        Field::HoursParallel => settings.hours_parallel as u64,
        Field::HoursInterval => settings.hours_interval_ms,
        Field::Duration => settings.duration_minutes,
    }
}

fn set_field(settings: &mut Settings, field: Field, value: u64) {
    match field {
        Field::VarietyParallel => settings.variety_parallel = value as usize,
        Field::VarietyInterval => settings.variety_interval_ms = value,
        Field::Hold => settings.hold_seconds = value,
        Field::Limit => settings.limit = value as usize,
        Field::HoursParallel => settings.hours_parallel = value as usize,
        Field::HoursInterval => settings.hours_interval_ms = value,
        Field::Duration => settings.duration_minutes = value,
    }
}

fn format_field(field: Field, value: u64) -> String {
    match field {
        Field::VarietyParallel | Field::HoursParallel => value.to_string(),
        Field::VarietyInterval | Field::HoursInterval => {
            if value >= 1000 && value % 1000 == 0 {
                format!("{} с", value / 1000)
            } else {
                format!("{value} мс")
            }
        }
        Field::Hold => {
            if value < 60 {
                format!("{value} с")
            } else if value % 60 == 0 {
                format!("{} мин", value / 60)
            } else {
                format!("{} мин {} с", value / 60, value % 60)
            }
        }
        Field::Limit => {
            if value == 0 {
                "все игры".into()
            } else {
                value.to_string()
            }
        }
        Field::Duration => {
            if value == 0 {
                "до «Стоп»".into()
            } else if value < 60 {
                format!("{value} мин")
            } else if value % 60 == 0 {
                format!("{} ч", value / 60)
            } else {
                format!("{} ч {} мин", value / 60, value % 60)
            }
        }
    }
}

fn clamp_settings(settings: &mut Settings) {
    for hours in [false, true] {
        for def in slider_defs(hours) {
            let value = get_field(settings, def.field).clamp(def.min, def.max);
            set_field(settings, def.field, value);
        }
    }
    if settings.mode != "hours" {
        settings.mode = "variety".into();
    }
}

#[derive(Default, Clone)]
struct Snapshot {
    running: bool,
    phase: String,
    active: usize,
    launched: u64,
    completed: u64,
    failed: u64,
    process_hours: f64,
    elapsed: u64,
    planned_seconds: u64,
    planned_games: u64,
    mode: String,
}

enum Action {
    None,
    Drag,
    Minimize,
    Close,
    Start,
    Stop,
    Clear,
    OpenFolder,
    OpenUrl(&'static str),
    Destroy,
}

struct Toast {
    text: String,
    color: u32,
    shown: Instant,
}
struct Ui {
    app: Arc<App>,
    hwnd: Handle,
    scale: f32,
    fonts: Fonts,
    settings: Settings,

    snap: Snapshot,
    last_elapsed: u64,
    games_total: usize,
    history_total: usize,
    played: usize,
    last_history_check: Instant,

    logs: VecDeque<(String, String)>,
    last_log_id: u64,
    log_scroll: Anim,

    hot: Hit,
    pressed: Hit,
    dragging: Option<usize>,
    tracking: bool,
    info: bool,

    hover: [Anim; 15],
    sliders: [Anim; 4],
    mode: Anim,
    content: Anim,
    toggle: Anim,
    run: Anim,
    stats: [Anim; 4],
    progress: Anim,
    toast_anim: Anim,
    closing_anim: Anim,

    toast: Option<Toast>,
    closing: bool,
    born: Instant,
    last_frame: Instant,
    last_paint: Instant,
}

impl Ui {
    fn new(app: Arc<App>, hwnd: Handle, scale: f32) -> Self {
        let mut settings = lock(&app.settings).clone();
        clamp_settings(&mut settings);
        let hours = settings.mode == "hours";
        let now = Instant::now();

        let mut ui = Self {
            app,
            hwnd,
            scale,
            fonts: Fonts::new(),
            snap: Snapshot::default(),
            last_elapsed: 0,
            games_total: 0,
            history_total: 0,
            played: 0,
            last_history_check: now,
            logs: VecDeque::new(),
            last_log_id: 0,
            log_scroll: Anim::new(0.0),
            hot: Hit::None,
            pressed: Hit::None,
            dragging: None,
            tracking: false,
            info: false,
            hover: [Anim::default(); 15],
            sliders: [Anim::default(); 4],
            mode: Anim::new(if hours { 1.0 } else { 0.0 }),
            content: Anim::new(0.0),
            toggle: Anim::new(if settings.skip_played { 1.0 } else { 0.0 }),
            run: Anim::new(0.0),
            stats: [Anim::default(); 4],
            progress: Anim::default(),
            toast_anim: Anim::default(),
            closing_anim: Anim::default(),
            toast: None,
            closing: false,
            born: now,
            last_frame: now,
            last_paint: now,
            settings,
        };

        ui.content.target = 1.0;
        ui.refresh_history_counts();
        ui
    }

    fn hours(&self) -> bool {
        self.settings.mode == "hours"
    }

    fn locked(&self) -> bool {
        self.snap.running || self.closing
    }

    fn show_toast(&mut self, text: impl Into<String>, color: u32) {
        self.toast = Some(Toast {
            text: text.into(),
            color,
            shown: Instant::now(),
        });
        self.toast_anim.target = 1.0;
    }

    fn refresh_history_counts(&mut self) {
        let history = lock(&self.app.history);
        self.history_total = history.games.len();
        self.played = history.games.values().filter(|e| e.completed).count();
        drop(history);
        self.games_total = *lock(&self.app.games_count);
        self.last_history_check = Instant::now();
    }

    fn pull_state(&mut self) {
        {
            let s = lock(&self.app.status);
            let elapsed = s.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            if s.running {
                self.last_elapsed = elapsed;
            }
            self.snap = Snapshot {
                running: s.running,
                phase: s.phase.clone(),
                active: s.active,
                launched: s.launched,
                completed: s.completed,
                failed: s.failed,
                process_hours: s.process_seconds / 3600.0,
                elapsed: self.last_elapsed,
                planned_seconds: s.planned_seconds,
                planned_games: s.planned_games,
                mode: s.mode.clone(),
            };
        }

        let mut added = 0usize;
        {
            let logs = lock(&self.app.logs);
            for (id, _, message) in logs.lines.iter() {
                if *id <= self.last_log_id {
                    continue;
                }
                self.last_log_id = *id;
                let mut time = LocalTime::default();
                unsafe {
                    GetLocalTime(&mut time);
                }
                let stamp = format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second);
                self.logs.push_back((stamp, message.clone()));
                added += 1;
            }
        }

        while self.logs.len() > MAX_UI_LOG {
            self.logs.pop_front();
        }

        // Если пользователь прокрутил журнал вверх, не сбиваем позицию.
        if added > 0 && self.log_scroll.target > 0.0 {
            self.log_scroll.target += added as f32;
            self.log_scroll.value += added as f32;
            self.clamp_scroll();
        }

        if self.last_history_check.elapsed().as_millis() > 500 {
            self.refresh_history_counts();
        }
    }

    fn visible_log_lines(&self) -> f32 {
        (LOG_H / LOG_LINE).floor()
    }

    fn clamp_scroll(&mut self) {
        let max = (self.logs.len() as f32 - self.visible_log_lines()).max(0.0);
        self.log_scroll.target = self.log_scroll.target.clamp(0.0, max);
        self.log_scroll.value = self.log_scroll.value.clamp(0.0, max);
    }

    fn tick(&mut self) -> (Action, bool) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.0, 0.1);
        self.last_frame = now;

        self.pull_state();

        // цели анимаций
        for (index, anim) in self.hover.iter_mut().enumerate() {
            let active = self.hot.hover_index() == Some(index)
                || self.dragging.map(|i| i + 4) == Some(index);
            anim.target = if active { 1.0 } else { 0.0 };
        }

        let defs = slider_defs(self.hours());
        for (index, def) in defs.iter().enumerate() {
            let value = get_field(&self.settings, def.field);
            self.sliders[index].target =
                (value - def.min) as f32 / (def.max - def.min).max(1) as f32;
        }

        self.mode.target = if self.info {
            2.0
        } else if self.hours() {
            1.0
        } else {
            0.0
        };
        self.toggle.target = if self.settings.skip_played { 1.0 } else { 0.0 };
        self.run.target = if self.snap.running { 1.0 } else { 0.0 };
        self.closing_anim.target = if self.closing { 1.0 } else { 0.0 };

        self.stats[0].target = self.snap.launched as f32;
        self.stats[1].target = self.snap.active as f32;
        self.stats[2].target = self.snap.completed as f32;
        self.stats[3].target = self.snap.process_hours as f32;

        self.progress.target = self.progress_fraction().unwrap_or(0.0);

        if let Some(toast) = &self.toast {
            if toast.shown.elapsed().as_secs_f32() > 3.5 {
                self.toast_anim.target = 0.0;
            }
        }

        let mut moving = false;
        for anim in self.hover.iter_mut() {
            moving |= anim.step(dt, 14.0);
        }
        for anim in self.sliders.iter_mut() {
            moving |= anim.step(dt, 16.0);
        }
        moving |= self.mode.step(dt, 12.0);
        moving |= self.content.step(dt, 8.0);
        moving |= self.toggle.step(dt, 14.0);
        moving |= self.run.step(dt, 8.0);
        for anim in self.stats.iter_mut() {
            moving |= anim.step(dt, 6.0);
        }
        moving |= self.progress.step(dt, 5.0);
        moving |= self.toast_anim.step(dt, 10.0);
        moving |= self.closing_anim.step(dt, 8.0);
        moving |= self.log_scroll.step(dt, 14.0);

        if self.toast_anim.target == 0.0 && self.toast_anim.value < 0.01 {
            self.toast = None;
        }

        let need_paint = moving
            || self.snap.running
            || self.closing
            || self.last_paint.elapsed().as_millis() > 500;

        if need_paint {
            self.last_paint = now;
        }

        if self.closing && !self.snap.running {
            return (Action::Destroy, need_paint);
        }

        if !self.closing && self.app.quit.load(Ordering::SeqCst) {
            return (Action::Close, need_paint);
        }

        (Action::None, need_paint)
    }

    fn progress_fraction(&self) -> Option<f32> {
        let s = &self.snap;
        if s.mode == "hours" {
            if s.planned_seconds == 0 {
                return None;
            }
            Some((s.elapsed as f32 / s.planned_seconds as f32).clamp(0.0, 1.0))
        } else if s.planned_games > 0 {
            Some((s.launched as f32 / s.planned_games as f32).clamp(0.0, 1.0))
        } else {
            Some(0.0)
        }
    }

    fn hit_test(&self, x: f32, y: f32) -> Hit {
        let inside = |rx: f32, ry: f32, rw: f32, rh: f32| {
            x >= rx && x < rx + rw && y >= ry && y < ry + rh
        };

        if inside(W - 52.0, 14.0, 36.0, 32.0) {
            return Hit::Close;
        }
        if inside(W - 94.0, 14.0, 36.0, 32.0) {
            return Hit::Min;
        }
        if y < 60.0 {
            return Hit::Title;
        }
        if inside(48.0, 118.0, 372.0, 34.0) {
            return Hit::Tab(((x - 48.0) / 124.0) as u8);
        }

        if self.info {
            if inside(44.0, 296.0, 184.0, 42.0) {
                return Hit::Github;
            }
            if inside(240.0, 296.0, 184.0, 42.0) {
                return Hit::Telegram;
            }
            if inside(464.0, 312.0, 492.0, 324.0) {
                return Hit::Log;
            }
            return Hit::None;
        }

        let count = slider_defs(self.hours()).len();
        for index in 0..count {
            let y0 = SL_Y0 + index as f32 * SL_STEP;
            if inside(SL_X - 10.0, y0, SL_W + 20.0, 52.0) {
                return Hit::Slider(index);
            }
        }

        if inside(44.0, 446.0, 380.0, 46.0) {
            return Hit::Toggle;
        }
        if inside(44.0, 506.0, 380.0, 52.0) {
            return Hit::Start;
        }
        if inside(44.0, 572.0, 184.0, 42.0) {
            return Hit::Clear;
        }
        if inside(240.0, 572.0, 184.0, 42.0) {
            return Hit::Folder;
        }
        if inside(464.0, 312.0, 492.0, 324.0) {
            return Hit::Log;
        }
        Hit::None
    }
    fn clickable(&self) -> bool {
        match self.hot {
            Hit::Min | Hit::Close | Hit::Folder | Hit::Github | Hit::Telegram | Hit::Tab(_) => true,
            Hit::Start => !self.closing,
            Hit::Slider(_) | Hit::Toggle | Hit::Clear => !self.locked(),
            _ => false,
        }
    }


    fn set_slider_from_x(&mut self, index: usize, x: f32) {
        let defs = slider_defs(self.hours());
        let Some(def) = defs.get(index) else {
            return;
        };
        let fraction = ((x - SL_X) / SL_W).clamp(0.0, 1.0) as f64;
        let raw = def.min as f64 + fraction * (def.max - def.min) as f64;
        let steps = ((raw - def.min as f64) / def.step as f64).round() as u64;
        let value = (def.min + steps * def.step).clamp(def.min, def.max);
        set_field(&mut self.settings, def.field, value);
    }

    fn on_move(&mut self, x: f32, y: f32) {
        if let Some(index) = self.dragging {
            self.set_slider_from_x(index, x);
        }

        self.hot = self.hit_test(x, y);

        if !self.tracking {
            let mut event = TrackMouse {
                size: std::mem::size_of::<TrackMouse>() as u32,
                flags: 0x2, // TME_LEAVE
                hwnd: self.hwnd,
                hover_time: 0,
            };
            unsafe {
                TrackMouseEvent(&mut event);
            }
            self.tracking = true;
        }
    }

    fn on_down(&mut self, x: f32, y: f32) -> Action {
        let hit = self.hit_test(x, y);
        self.hot = hit;

        match hit {
            Hit::Title => return Action::Drag,
            Hit::Slider(index) if !self.locked() => {
                self.dragging = Some(index);
                self.set_slider_from_x(index, x);
                unsafe {
                    SetCapture(self.hwnd);
                }
            }
            _ => self.pressed = hit,
        }
        Action::None
    }

    fn on_up(&mut self, x: f32, y: f32) -> Action {
        if self.dragging.take().is_some() {
            unsafe {
                ReleaseCapture();
            }
            return Action::None;
        }

        let hit = self.hit_test(x, y);
        let pressed = std::mem::replace(&mut self.pressed, Hit::None);

        if hit != pressed {
            return Action::None;
        }

        match hit {
            Hit::Min => Action::Minimize,
            Hit::Close => Action::Close,
            Hit::Tab(2) => {
                if !self.info {
                    self.info = true;
                    self.content = Anim { value: 0.0, target: 1.0 };
                }
                Action::None
            }
            Hit::Tab(tab) => {
                let mode = if tab == 1 { "hours" } else { "variety" };
                let mut changed = false;
                if self.info {
                    self.info = false;
                    changed = true;
                }
                if !self.locked() && self.settings.mode != mode {
                    self.settings.mode = mode.into();
                    changed = true;
                }
                if changed {
                    self.content = Anim { value: 0.0, target: 1.0 };
                    for slider in self.sliders.iter_mut() {
                        slider.value = 0.0;
                    }
                }
                Action::None
            }
            Hit::Github => Action::OpenUrl("https://github.com/ZigRain-hub"),
            Hit::Telegram => Action::OpenUrl("https://t.me/ZigRainZ"),
            Hit::Toggle if !self.locked() => {
                self.settings.skip_played = !self.settings.skip_played;
                Action::None
            }
            Hit::Start if !self.closing => {
                if self.snap.running {
                    Action::Stop
                } else {
                    Action::Start
                }
            }
            Hit::Clear if !self.locked() => Action::Clear,
            Hit::Folder => Action::OpenFolder,
            _ => Action::None,
        }
    }

    fn on_wheel(&mut self, delta: i16) {
        let notches = delta as f32 / 120.0;
        let hot = self.hot;
        match hot {
            Hit::Slider(index) if !self.locked() => {
                let defs = slider_defs(self.hours());
                if let Some(def) = defs.get(index) {
                    let value = get_field(&self.settings, def.field) as i64;
                    let next = (value + (notches.round() as i64) * def.step as i64)
                        .clamp(def.min as i64, def.max as i64);
                    set_field(&mut self.settings, def.field, next as u64);
                }
            }
            Hit::Log => {
                self.log_scroll.target += notches * 3.0;
                self.clamp_scroll();
            }
            _ => {}
        }
    }

    fn on_leave(&mut self) {
        self.tracking = false;
        if self.dragging.is_none() {
            self.hot = Hit::None;
        }
    }

    fn render(&self, hdc: Handle) {
        let mut rect = Rect::default();
        unsafe {
            GetClientRect(self.hwnd, &mut rect);
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return;
        }

        unsafe {
            // двойная буферизация: рисуем в память, затем копируем целиком
            let mem = CreateCompatibleDC(hdc);
            let bitmap = CreateCompatibleBitmap(hdc, width, height);
            if mem.is_null() || bitmap.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                if !mem.is_null() {
                    DeleteDC(mem);
                }
                return;
            }
            let old = SelectObject(mem, bitmap);

            let mut graphics: Gp = ptr::null_mut();
            if GdipCreateFromHDC(mem, &mut graphics) == 0 && !graphics.is_null() {
                GdipSetSmoothingMode(graphics, 4); // AntiAlias
                GdipSetPixelOffsetMode(graphics, 4); // Half
                GdipSetTextRenderingHint(graphics, 5); // ClearTypeGridFit
                GdipGraphicsClear(graphics, BG);
                GdipScaleWorldTransform(graphics, self.scale, self.scale, 0);

                let mut painter = Painter { g: graphics, alpha: 1.0 };
                self.draw(&mut painter);
                GdipDeleteGraphics(graphics);
            }

            BitBlt(hdc, 0, 0, width, height, mem, 0, 0, 0x00CC_0020);
            SelectObject(mem, old);
            DeleteObject(bitmap);
            DeleteDC(mem);
        }
    }

    fn time(&self) -> f32 {
        self.born.elapsed().as_secs_f32()
    }

    fn draw(&self, p: &mut Painter) {
        let t = self.time();

        for ring in 0..10 {
            let r = 340.0 - ring as f32 * 26.0;
            p.circle(150.0, 40.0, r, with_alpha(ACCENT_A, 0.018));
            p.circle(W - 90.0, H + 40.0, r * 0.9, with_alpha(ACCENT_B, 0.014));
        }

        self.draw_title(p, t);

        p.fill_rr(24.0, 76.0, 420.0, 560.0, 18.0, CARD);
        p.stroke_rr(24.0, 76.0, 420.0, 560.0, 18.0, CARD_BORDER, 1.0);
        self.draw_controls(p);
        self.draw_stats(p, t);
        self.draw_log(p);

        p.stroke_rr(0.5, 0.5, W - 1.0, H - 1.0, 8.0, CARD_BORDER, 1.0);

        self.draw_toast(p);
        self.draw_closing(p, t);

    }

    fn draw_title(&self, p: &mut Painter, t: f32) {
        let f = &self.fonts;

        p.fill_rr_grad(24.0, 14.0, 34.0, 34.0, 10.0, ACCENT_A, ACCENT_B);
        p.polygon(
            &[
                PointF { x: 36.0, y: 23.0 },
                PointF { x: 36.0, y: 39.0 },
                PointF { x: 49.0, y: 31.0 },
            ],
            0xF0FFFFFF,
        );

        p.text("QDCord", f.title, f.near, 70.0, 12.0, 300.0, 22.0, TEXT);
        p.text(
            "Имитация игровой активности для Discord",
            f.small,
            f.near,
            70.0,
            34.0,
            360.0,
            16.0,
            MUTED,
        );

        let (label, color) = if self.closing {
            ("Остановка…".to_string(), YELLOW)
        } else if self.snap.running {
            (format!("Работает · {} акт.", self.snap.active), GREEN)
        } else {
            ("Ожидание".to_string(), MUTED)
        };

        let px = W - 290.0;
        p.fill_rr(px, 17.0, 180.0, 28.0, 14.0, SURFACE);
        p.stroke_rr(px, 17.0, 180.0, 28.0, 14.0, with_alpha(color, 0.35), 1.0);

        let cx = px + 18.0;
        if self.snap.running || self.closing {
            let phase = (t * 1.2) % 1.0;
            p.circle(cx, 31.0, 4.0 + phase * 7.0, with_alpha(color, 0.45 * (1.0 - phase)));
        }
        p.circle(cx, 31.0, 4.0, color);
        p.text(&label, f.small, f.near, px + 30.0, 17.0, 146.0, 28.0, TEXT);

        let min_h = self.hover[0].value;
        let close_h = self.hover[1].value;
        p.fill_rr(W - 94.0, 14.0, 36.0, 32.0, 8.0, with_alpha(0xFFFFFFFF, 0.08 * min_h));
        p.fill_rr(W - 52.0, 14.0, 36.0, 32.0, 8.0, with_alpha(RED, 0.9 * close_h));
        p.line(W - 82.0, 30.0, W - 70.0, 30.0, mix(MUTED, TEXT, min_h), 1.6);
        let close_color = mix(MUTED, 0xFFFFFFFF, close_h);
        p.line(W - 40.0, 24.0, W - 28.0, 36.0, close_color, 1.6);
        p.line(W - 28.0, 24.0, W - 40.0, 36.0, close_color, 1.6);
    }


    fn draw_controls(&self, p: &mut Painter) {
        let f = &self.fonts;
        let locked = self.locked();
        let dim = if locked { 0.45 } else { 1.0 };

        p.text("РАЗДЕЛ", f.small, f.near, 44.0, 90.0, 200.0, 20.0, MUTED);

        p.alpha = if self.info { 1.0 } else { dim };
        p.fill_rr(44.0, 114.0, 380.0, 42.0, 12.0, SURFACE);
        let ix = 48.0 + self.mode.value * 124.0;
        p.fill_rr_grad(ix, 118.0, 124.0, 34.0, 9.0, ACCENT_A, ACCENT_B);
        for (index, label) in ["Игры", "Часы", "Справочник"].iter().enumerate() {
            let selected = 1.0 - (self.mode.value - index as f32).abs().min(1.0);
            let slot = if index < 2 { 2 + index } else { 12 };
            let hover = self.hover[slot].value * (1.0 - selected);
            let color = mix(mix(MUTED, TEXT, hover), 0xFFFFFFFF, selected);
            p.text(label, f.body_bold, f.center, 48.0 + index as f32 * 124.0, 118.0, 124.0, 34.0, color);
        }

        // Содержимое вкладки проявляется и слегка сдвигается.
        let fade = self.content.value;
        let shift = (1.0 - fade) * 14.0;
        p.alpha = dim * fade;

        if self.info {
            p.alpha = fade;
            self.draw_info(p, shift);
            p.alpha = 1.0;
            return;
        }

        let defs = slider_defs(self.hours());
        for (index, def) in defs.iter().enumerate() {
            let y0 = SL_Y0 + index as f32 * SL_STEP + shift;
            let value = get_field(&self.settings, def.field);
            let hover = self.hover[4 + index].value;
            let pos = self.sliders[index].value.clamp(0.0, 1.0);

            p.text(def.label, f.body, f.near, SL_X, y0, 240.0, 22.0, TEXT);
            p.text(
                &format_field(def.field, value),
                f.body_bold,
                f.far,
                SL_X + 160.0,
                y0,
                SL_W - 160.0,
                22.0,
                mix(TEXT, ACCENT_B, hover),
            );

            let ty = y0 + 36.0;
            p.fill_rr(SL_X, ty - 3.0, SL_W, 6.0, 3.0, BUTTON);
            p.fill_rr_grad(SL_X, ty - 3.0, (SL_W * pos).max(6.0), 6.0, 3.0, ACCENT_A, ACCENT_B);

            let cx = SL_X + SL_W * pos;
            let dragging = self.dragging == Some(index);
            p.circle(cx, ty, 9.0 + hover * 7.0, with_alpha(ACCENT_B, 0.18 * hover));
            p.circle(cx, ty, 8.0 + hover * 1.5 + if dragging { 1.0 } else { 0.0 }, 0xFFFFFFFF);
            p.circle(cx, ty, 3.0, mix(ACCENT_A, ACCENT_B, pos));
        }

        if self.hours() {
            let y0 = SL_Y0 + 3.0 * SL_STEP + shift;
            p.fill_rr(SL_X, y0, SL_W, 54.0, 12.0, with_alpha(ACCENT_A, 0.10));
            p.stroke_rr(SL_X, y0, SL_W, 54.0, 12.0, with_alpha(ACCENT_A, 0.35), 1.0);
            let text = format!(
                "До {} процессо-часов за 1 час реального времени. Discord может засчитать не всё.",
                self.settings.hours_parallel
            );
            p.text(&text, f.small, f.wrap, SL_X + 14.0, y0 + 10.0, SL_W - 28.0, 40.0, TEXT);
        }

        p.alpha = dim;

        // переключатель «пропускать»
        let tv = self.toggle.value;
        let th = self.hover[8].value;
        p.text("Пропускать игры из истории", f.body, f.near, 44.0, 448.0, 300.0, 22.0, TEXT);
        p.text("Уже отработавшие по history.json", f.small, f.near, 44.0, 470.0, 300.0, 18.0, MUTED);
        let track = mix(mix(BUTTON, 0xFF2E3344, th), ACCENT_A, tv);
        p.fill_rr(380.0, 457.0, 44.0, 24.0, 12.0, track);
        p.circle(392.0 + tv * 20.0, 469.0, 9.0, 0xFFFFFFFF);

        let ch = self.hover[10].value;
        p.fill_rr(44.0, 572.0, 184.0, 42.0, 12.0, mix(BUTTON, 0xFF2E3344, ch));
        p.text("Очистить историю", f.body, f.center, 44.0, 572.0, 184.0, 42.0, TEXT);

        p.alpha = 1.0;
        let fh = self.hover[11].value;
        p.fill_rr(240.0, 572.0, 184.0, 42.0, 12.0, mix(BUTTON, 0xFF2E3344, fh));
        p.text("Открыть папку", f.body, f.center, 240.0, 572.0, 184.0, 42.0, TEXT);

        let r = self.run.value;
        let sh = self.hover[9].value;
        p.alpha = if self.closing { 0.5 } else { 1.0 };
        let lift = sh * 2.0;
        p.fill_rr(52.0, 514.0 + 4.0, 364.0, 48.0, 14.0, with_alpha(mix(ACCENT_A, RED, r), 0.25 + 0.2 * sh));
        p.fill_rr_grad(44.0, 506.0 - lift, 380.0, 52.0, 14.0, mix(ACCENT_A, RED, r), mix(ACCENT_B, RED_B, r));
        p.fill_rr(44.0, 506.0 - lift, 380.0, 52.0, 14.0, with_alpha(0xFFFFFFFF, 0.10 * sh));

        let label = if self.snap.running { "Остановить" } else { "Запустить" };
        let cx = 44.0 + 190.0 - 62.0;
        let cy = 532.0 - lift;
        if self.snap.running {
            p.fill_rr(cx - 7.0, cy - 7.0, 14.0, 14.0, 3.0, 0xFFFFFFFF);
        } else {
            p.polygon(
                &[
                    PointF { x: cx - 6.0, y: cy - 8.0 },
                    PointF { x: cx - 6.0, y: cy + 8.0 },
                    PointF { x: cx + 8.0, y: cy },
                ],
                0xFFFFFFFF,
            );
        }
        p.text(label, f.heading, f.near, cx + 18.0, 506.0 - lift, 200.0, 52.0, 0xFFFFFFFF);
        p.alpha = 1.0;
    }

    fn draw_info(&self, p: &mut Painter, shift: f32) {
        let f = &self.fonts;
        let y = shift;

        p.text("QDCord", f.heading, f.near, 44.0, 176.0 + y, 200.0, 28.0, TEXT);
        p.text(&format!("версия {}", VERSION), f.small, f.far, 224.0, 182.0 + y, 200.0, 20.0, MUTED);

        p.text(
            "Запускает безобидные процессы-заглушки с именами игр из detectable-games.json, а Discord сам видит их среди запущенных программ. Токены и запросы к Discord API не используются, клиент не изменяется.",
            f.small,
            f.wrap,
            44.0,
            212.0 + y,
            380.0,
            76.0,
            MUTED,
        );

        for (index, (label, x)) in [("GitHub", 44.0f32), ("Telegram", 240.0f32)].iter().enumerate() {
            let h = self.hover[13 + index].value;
            p.fill_rr(*x, 296.0 + y, 184.0, 42.0, 12.0, mix(BUTTON, 0xFF2E3344, h));
            p.text(label, f.body, f.center, *x, 296.0 + y, 184.0, 42.0, mix(TEXT, ACCENT_B, h));
        }
        p.text("github.com/ZigRain-hub  ·  t.me/ZigRainZ", f.small, f.center, 44.0, 346.0 + y, 380.0, 18.0, MUTED);

        p.text("ОТКАЗ ОТ ОТВЕТСТВЕННОСТИ", f.small, f.near, 44.0, 384.0 + y, 380.0, 18.0, MUTED);
        p.fill_rr(44.0, 408.0 + y, 380.0, 150.0, 12.0, with_alpha(YELLOW, 0.08));
        p.stroke_rr(44.0, 408.0 + y, 380.0, 150.0, 12.0, with_alpha(YELLOW, 0.35), 1.0);
        p.text(
            "Разработчик не несёт никакой ответственности за использование программы и любые последствия: блокировку аккаунта, нагрузку на компьютер, потерю данных. Программа распространяется «как есть», без гарантий. Discord может не засчитать запуски. Используя QDCord, вы действуете на свой риск и сами отвечаете за соблюдение правил Discord.",
            f.small,
            f.wrap,
            58.0,
            420.0 + y,
            352.0,
            132.0,
            TEXT,
        );

        p.text("Файлы рядом с программой: settings.json, history.json, qdcord.log", f.small, f.wrap, 44.0, 572.0 + y, 380.0, 36.0, MUTED);
    }

    fn draw_stats(&self, p: &mut Painter, t: f32) {
        let f = &self.fonts;
        let labels = ["Запущено", "Активно", "Отработало", "Процессо-часы"];
        let colors = [ACCENT_A, GREEN, ACCENT_B, YELLOW];

        for index in 0..4 {
            let x = 464.0 + index as f32 * 126.0;
            p.fill_rr(x, 76.0, 114.0, 100.0, 16.0, CARD);
            p.stroke_rr(x, 76.0, 114.0, 100.0, 16.0, CARD_BORDER, 1.0);
            p.fill_rr(x + 14.0, 90.0, 18.0, 4.0, 2.0, colors[index]);
            p.text(labels[index], f.small, f.near, x + 14.0, 98.0, 96.0, 20.0, MUTED);

            let value = self.stats[index].value;
            let text = if index == 3 {
                format!("{value:.1}")
            } else {
                format!("{}", value.round() as u64)
            };
            p.text(&text, f.big, f.near, x + 12.0, 120.0, 98.0, 42.0, TEXT);
        }

        p.fill_rr(464.0, 192.0, 492.0, 104.0, 16.0, CARD);
        p.stroke_rr(464.0, 192.0, 492.0, 104.0, 16.0, CARD_BORDER, 1.0);

        let s = &self.snap;
        let phase = if s.phase.is_empty() { "Ожидание" } else { s.phase.as_str() };
        p.text(phase, f.body_bold, f.near, 480.0, 204.0, 300.0, 24.0, TEXT);

        let timing = if s.mode == "hours" && s.planned_seconds > 0 {
            format!("{} / {}", fmt_duration(s.elapsed), fmt_duration(s.planned_seconds))
        } else if s.mode == "variety" && s.planned_games > 0 {
            format!("{} / {} игр · {}", s.launched, s.planned_games, fmt_duration(s.elapsed))
        } else {
            fmt_duration(s.elapsed)
        };
        p.text(&timing, f.body, f.far, 700.0, 204.0, 240.0, 24.0, MUTED);

        let (bx, by, bw, bh) = (480.0, 240.0, 460.0, 10.0);
        p.fill_rr(bx, by, bw, bh, 5.0, BUTTON);

        if s.running && self.progress_fraction().is_none() {
            let seg = 140.0;
            let pos = (t * 0.45) % 1.0 * (bw + seg) - seg;
            p.clip(bx, by, bw, bh);
            p.fill_rr_grad(bx + pos, by, seg, bh, 5.0, with_alpha(ACCENT_A, 0.0), ACCENT_B);
            p.unclip();
        } else {
            let fill = bw * self.progress.value.clamp(0.0, 1.0);
            if fill > 1.0 {
                p.fill_rr_grad(bx, by, fill.max(bh), bh, 5.0, ACCENT_A, ACCENT_B);
                if s.running {
                    let sweep = (t * 0.6) % 1.0 * (fill + 80.0) - 80.0;
                    p.clip(bx, by, fill, bh);
                    p.fill_rr_grad(bx + sweep, by, 80.0, bh, 5.0, 0x00FFFFFF, 0x55FFFFFF);
                    p.unclip();
                }
            }
        }

        let info = format!(
            "В базе: {}  ·  в истории: {}  ·  отработало: {}  ·  ошибок: {}",
            self.games_total, self.history_total, self.played, s.failed
        );
        p.text(&info, f.small, f.near, 480.0, 262.0, 460.0, 22.0, MUTED);
    }

    fn draw_log(&self, p: &mut Painter) {
        let f = &self.fonts;
        p.fill_rr(464.0, 312.0, 492.0, 324.0, 16.0, CARD);
        p.stroke_rr(464.0, 312.0, 492.0, 324.0, 16.0, CARD_BORDER, 1.0);
        p.text("Журнал", f.heading, f.near, 480.0, 322.0, 200.0, 26.0, TEXT);
        p.text(
            &format!("{} записей · колесо мыши для прокрутки", self.logs.len()),
            f.small,
            f.far,
            640.0,
            322.0,
            300.0,
            26.0,
            MUTED,
        );

        let visible = self.visible_log_lines();
        let total = self.logs.len() as f32;
        let start = ((total - visible).max(0.0) - self.log_scroll.value).max(0.0);

        p.clip(LOG_X, LOG_Y, LOG_W, LOG_H);
        let first = start.floor() as usize;
        let last = ((start + visible + 1.0).ceil() as usize).min(self.logs.len());
        for index in first..last {
            let (time, message) = &self.logs[index];
            let y = LOG_Y + (index as f32 - start) * LOG_LINE;
            let color = if message.contains("ОШИБКА") {
                RED
            } else if message.contains("ПРЕДУПРЕЖДЕНИЕ") {
                YELLOW
            } else if message.starts_with("Запущен") {
                mix(TEXT, ACCENT_B, 0.35)
            } else {
                0xFFC3C7D4
            };
            p.text(time, f.mono, f.near, LOG_X, y, 70.0, LOG_LINE, MUTED);
            p.text(message, f.mono, f.near, LOG_X + 70.0, y, LOG_W - 80.0, LOG_LINE, color);
        }

        if start > 0.0 {
            p.fill_rect_vgrad(LOG_X, LOG_Y, LOG_W, 22.0, CARD, CARD & 0x00FF_FFFF);
        }
        p.unclip();

        // полоса прокрутки.
        if total > visible {
            let track_h = LOG_H;
            let thumb_h = (track_h * visible / total).max(24.0);
            let pos = start / (total - visible);
            let ty = LOG_Y + (track_h - thumb_h) * pos;
            let alpha = 0.25 + 0.35 * if self.hot == Hit::Log { 1.0 } else { 0.0 };
            p.fill_rr(LOG_X + LOG_W - 4.0, ty, 4.0, thumb_h, 2.0, with_alpha(0xFFFFFFFF, alpha));
        }
    }

    fn draw_toast(&self, p: &mut Painter) {
        let Some(toast) = &self.toast else {
            return;
        };
        let a = self.toast_anim.value;
        if a < 0.01 {
            return;
        }
        let f = &self.fonts;
        let (w, h) = (460.0, 44.0);
        let x = (W - w) / 2.0;
        let y = H - 14.0 - h * a - (1.0 - a) * 6.0;

        p.alpha = a;
        p.fill_rr(x, y + 4.0, w, h, 12.0, 0x60000000);
        p.fill_rr(x, y, w, h, 12.0, 0xFF232837);
        p.stroke_rr(x, y, w, h, 12.0, with_alpha(toast.color, 0.6), 1.0);
        p.circle(x + 20.0, y + h / 2.0, 5.0, toast.color);
        p.text(&toast.text, f.body, f.near, x + 34.0, y, w - 46.0, h, TEXT);
        p.alpha = 1.0;
    }

    fn draw_closing(&self, p: &mut Painter, t: f32) {
        let a = self.closing_anim.value;
        if a < 0.01 {
            return;
        }
        let f = &self.fonts;
        p.alpha = a;
        p.fill_rr(0.0, 0.0, W, H, 0.0, 0xC00A0B10);

        let (cx, cy) = (W / 2.0, H / 2.0 - 20.0);
        p.arc(cx, cy, 22.0, 0.0, 360.0, 0x30FFFFFF, 4.0);
        p.arc(cx, cy, 22.0, (t * 320.0) % 360.0, 90.0, ACCENT_B, 4.0);
        p.text(
            "Останавливаем процессы…",
            f.heading,
            f.center,
            0.0,
            cy + 34.0,
            W,
            28.0,
            TEXT,
        );
        p.alpha = 1.0;
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

fn mouse_pos(lparam: isize, scale: f32) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as f32;
    (x / scale, y / scale)
}


fn handle_action(hwnd: Handle, action: Action) {
    match action {
        Action::None => {}
        Action::Drag => unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
        },
        Action::Minimize => unsafe {
            ShowWindow(hwnd, 6);
        },
        Action::Close => unsafe {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        },
        Action::Destroy => unsafe {
            DestroyWindow(hwnd);
        },
        Action::Stop => {
            if let Some(app) = with_ui(|ui| Arc::clone(&ui.app)) {
                app.stop.store(true, Ordering::SeqCst);
                app.log("Запрошена остановка.");
            }
        }
        Action::Start => {
            let Some((app, mut settings)) =
                with_ui(|ui| (Arc::clone(&ui.app), ui.settings.clone()))
            else {
                return;
            };
            clamp_settings(&mut settings);

            if settings.mode == "hours"
                && settings.hours_parallel > 64
                && !confirm(
                    hwnd,
                    &format!(
                        "Запустить {} процессов одновременно?\n\nЭто может заметно нагрузить компьютер.",
                        settings.hours_parallel
                    ),
                )
            {
                return;
            }

            let result = start_engine(&app, settings);
            with_ui(|ui| match result {
                Ok(()) => ui.show_toast("Запуск начат", GREEN),
                Err(error) => ui.show_toast(error, RED),
            });
        }
        Action::Clear => {
            let Some(app) = with_ui(|ui| Arc::clone(&ui.app)) else {
                return;
            };
            if !confirm(hwnd, "Очистить history.json?\n\nПрограмма снова будет запускать все игры.") {
                return;
            }
            let message = if lock(&app.status).running {
                Err("Сначала остановите запуск")
            } else {
                *lock(&app.history) = History { version: 1, ..History::default() };
                save_history(&app);
                app.log("История очищена.");
                Ok("История очищена")
            };
            with_ui(|ui| {
                ui.refresh_history_counts();
                match message {
                    Ok(text) => ui.show_toast(text, GREEN),
                    Err(text) => ui.show_toast(text, RED),
                }
            });
        }
        Action::OpenFolder => {
            if let Some(dir) = with_ui(|ui| ui.app.dir.clone()) {
                let _ = Command::new("explorer.exe").arg(dir).spawn();
            }
        }
        Action::OpenUrl(url) => {
            let _ = Command::new("explorer.exe").arg(url).spawn();
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PaintStruct = unsafe { std::mem::zeroed() };
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_null() {
                with_ui(|ui| ui.render(hdc));
            }
            unsafe {
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_TIMER => {
            if let Some((action, paint)) = with_ui(|ui| ui.tick()) {
                if paint {
                    unsafe {
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                handle_action(hwnd, action);
            }
            0
        }
        WM_MOUSEMOVE => {
            with_ui(|ui| {
                let (x, y) = mouse_pos(lparam, ui.scale);
                ui.on_move(x, y);
            });
            0
        }
        WM_LBUTTONDOWN => {
            let action = with_ui(|ui| {
                let (x, y) = mouse_pos(lparam, ui.scale);
                ui.on_down(x, y)
            });
            if let Some(action) = action {
                handle_action(hwnd, action);
            }
            0
        }
        WM_LBUTTONUP => {
            let action = with_ui(|ui| {
                let (x, y) = mouse_pos(lparam, ui.scale);
                ui.on_up(x, y)
            });
            if let Some(action) = action {
                handle_action(hwnd, action);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16;
            with_ui(|ui| ui.on_wheel(delta));
            0
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| ui.on_leave());
            0
        }
        WM_SETCURSOR => {
            let over_client = (lparam & 0xFFFF) == HTCLIENT;
            if over_client && with_ui(|ui| ui.clickable()).unwrap_or(false) {
                unsafe {
                    // 32649 = IDC_HAND.
                    SetCursor(LoadCursorW(ptr::null_mut(), 32649usize as *const u16));
                }
                return 1;
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CLOSE => {
            //если идёт запуск, сначала аккуратно останавливаем процессы
            let destroy = with_ui(|ui| {
                ui.app.stop.store(true, Ordering::SeqCst);
                ui.app.quit.store(true, Ordering::SeqCst);
                if lock(&ui.app.status).running {
                    ui.closing = true;
                    false
                } else {
                    true
                }
            })
            .unwrap_or(true);

            if destroy {
                unsafe {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_DESTROY => {
            with_ui(|ui| {
                let mut settings = ui.settings.clone();
                clamp_settings(&mut settings);
                *lock(&ui.app.settings) = settings;
                save_settings(&ui.app);
            });
            unsafe {
                KillTimer(hwnd, TIMER_ID);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn run_gui(app: Arc<App>) -> Result<(), String> {
    unsafe {
        SetProcessDPIAware();
    }

    let mut token: usize = 0;
    let input = GdipStartupInput {
        version: 1,
        callback: ptr::null(),
        suppress_background_thread: 0,
        suppress_external_codecs: 0,
    };
    let status = unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) };
    if status != 0 {
        return Err(format!("Не удалось запустить GDI+ (код {status})"));
    }

    let result = create_and_run(app);

    UI.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard.take();
        }
    });

    unsafe {
        GdiplusShutdown(token);
    }
    result
}

fn create_and_run(app: Arc<App>) -> Result<(), String> {
    let scale = unsafe {
        let screen = GetDC(ptr::null_mut());
        let dpi = if screen.is_null() { 96 } else { GetDeviceCaps(screen, 88) };
        if !screen.is_null() {
            ReleaseDC(ptr::null_mut(), screen);
        }
        (dpi.max(96) as f32) / 96.0
    };

    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class_name = wide("QDCordUi");
    let title = wide("QDCord");

    let class = WndClassW {
        style: 0,
        wnd_proc,
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: ptr::null_mut(),
        // 32512 = IDC_ARROW
        cursor: unsafe { LoadCursorW(ptr::null_mut(), 32512usize as *const u16) },
        background: ptr::null_mut(),
        menu_name: ptr::null(),
        class_name: class_name.as_ptr(),
    };

    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("Не удалось зарегистрировать класс окна".into());
    }

    let width = (W * scale).round() as i32;
    let height = (H * scale).round() as i32;
    let (screen_w, screen_h) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            ((screen_w - width) / 2).max(0),
            ((screen_h - height) / 2).max(0),
            width,
            height,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        return Err("Не удалось создать окно".into());
    }

    unsafe {
        // Windows 11: скруглённые углы и тёмная рамка. На Windows 10 просто игнорируется
        let round: u32 = 2;
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const c_void, 4);
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const c_void, 4);
    }

    UI.with(|cell| {
        *cell.borrow_mut() = Some(Ui::new(app, hwnd, scale));
    });

    unsafe {
        SetTimer(hwnd, TIMER_ID, 15, ptr::null());
        ShowWindow(hwnd, 5);
        UpdateWindow(hwnd);
    }

    let mut message: Msg = unsafe { std::mem::zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == 0 {
            break;
        }
        if result == -1 {
            return Err("Ошибка цикла сообщений".into());
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}
