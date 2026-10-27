mod animation;
mod dialogs;
mod graphics;
mod highlights;
mod input;
mod mouse;
pub mod page_animation;
mod render;
mod session;
mod widgets;

use dialogs::{
    annotations as annotation_render, dictionary as dictionary_render, hyperlinks,
    library as library_render, markdown_export as link_render,
    reassociation as reassociation_render, search as search_render,
};

use crate::Result;
use crate::app::Viewport;
use crate::app::runtime::{ReaderLaunch, ReaderRuntime};
use crate::book::BookLocation;
use crate::configuration::{Command, Configuration, InputKey};
use animation::RunningAnimation;
use crossterm::event::{Event, KeyEventKind};
use crossterm::{event, terminal};
use graphics::{inline as inline_images, preview as images, support as image_support};
use image_support::ImageSupportDetector;
use input::input_key;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use render::draw;
use session::{KeyboardMode, Signals, TerminalModes, TerminalSession, restore_terminal};
use std::io::stdout;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_millis(10);
const MINIMUM_COLUMNS: u16 = 22;
const MINIMUM_ROWS: u16 = 12;

struct TerminalLaunch {
    path: BookLocation,
    database: PathBuf,
    configuration: Configuration,
}

pub fn read_book(
    path: BookLocation,
    database: PathBuf,
    configuration: Configuration,
) -> Result<()> {
    let previous_hook = Arc::new(std::panic::take_hook());
    let panic_hook = previous_hook.clone();
    let modes = Arc::new(std::sync::Mutex::new(TerminalModes::default()));
    let panic_modes = modes.clone();
    std::panic::set_hook(Box::new(move |information| {
        restore_terminal(&panic_modes);
        panic_hook(information);
    }));
    let result = reader_loop(
        TerminalLaunch {
            path,
            database,
            configuration,
        },
        modes,
    );
    std::panic::set_hook(Box::new(move |information| previous_hook(information)));
    result
}

fn viewport(columns: u16, rows: u16) -> Viewport {
    Viewport {
        columns: columns as usize,
        rows: (rows as usize).saturating_sub(2).max(1),
    }
}

fn usable_size(columns: u16, rows: u16) -> bool {
    columns >= MINIMUM_COLUMNS && rows >= MINIMUM_ROWS
}

fn wait_for_usable_size(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    signals: &Signals,
    configuration: &Configuration,
) -> Result<Option<(u16, u16)>> {
    loop {
        let (columns, rows) = terminal::size()?;
        if usable_size(columns, rows) {
            return Ok(Some((columns, rows)));
        }
        terminal.draw(render::draw_small_terminal)?;
        if !wait_for_resize_or_quit(signals, configuration)? {
            return Ok(None);
        }
    }
}

fn wait_for_resize_or_quit(signals: &Signals, configuration: &Configuration) -> Result<bool> {
    loop {
        if signals.requested.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Resize(_, _) => return Ok(true),
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(key) = input_key(key)
                    && (key == InputKey::Character('\u{3}')
                        || configuration.bindings.command_for(key) == Some(Command::Quit))
                {
                    return Ok(false);
                }
            }
            _ => {}
        }
    }
}

struct IntegrationDisplay<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<std::io::Stdout>>,
    runtime: &'a mut ReaderRuntime,
    session: &'a TerminalSession,
    mouse: bool,
    image_support: &'a ImageSupportDetector,
    preview: &'a mut images::ImageRenderer,
    inline: &'a mut inline_images::InlineImages,
}

impl IntegrationDisplay<'_> {
    fn run(&mut self, operation: impl FnOnce() -> Result<()>) -> Result<ratatui::layout::Size> {
        self.preview.clear();
        self.inline.clear();
        self.preview.flush_cleanup(&mut stdout())?;
        self.inline.flush_cleanup(&mut stdout())?;
        let result = self.session.suspend(self.mouse, operation)?;
        self.terminal.clear()?;
        let (columns, rows) = terminal::size()?;
        self.preview.set_support(self.image_support.detect());
        if usable_size(columns, rows) {
            self.runtime.resize(viewport(columns, rows));
        }
        self.runtime
            .integration_completed(result.map_err(|error| error.to_string()));
        Ok(ratatui::layout::Size::new(columns, rows))
    }
}

fn reader_loop(launch: TerminalLaunch, modes: Arc<std::sync::Mutex<TerminalModes>>) -> Result<()> {
    let TerminalLaunch {
        path,
        database,
        configuration,
    } = launch;
    let mouse = configuration.settings.mouse_support;
    let browser_preference = std::env::var("BROWSER").ok();
    let viewer_preference = configuration.settings.default_viewer.clone();
    let mut image_viewer = crate::integrations::ImageViewer::default();
    let keyboard = if std::env::var_os("TMUX").is_some_and(|value| !value.is_empty()) {
        KeyboardMode::Tmux
    } else {
        KeyboardMode::Kitty
    };
    let session = TerminalSession::enter(mouse, keyboard, modes)?;
    let signals = Signals::register()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let Some((columns, rows)) = wait_for_usable_size(&mut terminal, &signals, &configuration)?
    else {
        return Ok(());
    };
    let image_decoder = std::env::current_exe()?;
    let mut runtime = ReaderRuntime::start(ReaderLaunch {
        image_decoder: image_decoder.clone(),
        executables: crate::integrations::ExecutableSearch::capture()?,
        location: path,
        database,
        configuration,
        viewport: viewport(columns, rows),
    });
    let mut screen_size = ratatui::layout::Size::new(columns, rows);
    let image_support = ImageSupportDetector::new();
    let support = image_support.detect();
    let image_ids = images::ImageIds::new(uuid::Uuid::new_v4().as_u128() as u32);
    let mut image_renderer = images::ImageRenderer::new(support, image_ids.clone());
    let mut inline_images = inline_images::InlineImages::new(
        crate::images::ImageWorkerOptions::new(image_decoder),
        support,
        image_ids,
    );
    let mut dirty = true;
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut last_input = Instant::now();
    let mut animation = None::<RunningAnimation>;
    let mut mouse_tracker = mouse::MouseTracker::default();
    let result = (|| {
        loop {
            dirty |= runtime.poll()?;
            if usable_size(screen_size.width, screen_size.height) {
                dirty |= image_renderer.sync(runtime.model.as_ref(), screen_size);
                inline_images.set_support(image_renderer.support());
                dirty |= inline_images.sync(
                    runtime.model.as_ref(),
                    screen_size,
                    runtime.book_location(),
                );
            }
            image_renderer.flush_cleanup(&mut stdout())?;
            inline_images.flush_cleanup(&mut stdout())?;
            while let Some(url) = runtime.take_browser_request() {
                animation = None;
                screen_size = IntegrationDisplay {
                    terminal: &mut terminal,
                    runtime: &mut runtime,
                    session: &session,
                    mouse,
                    image_support: &image_support,
                    preview: &mut image_renderer,
                    inline: &mut inline_images,
                }
                .run(|| crate::integrations::open_browser(&url, browser_preference.as_deref()))?;
                dirty = true;
            }
            while let Some(payload) = runtime.take_viewer_request() {
                animation = None;
                screen_size = IntegrationDisplay {
                    terminal: &mut terminal,
                    runtime: &mut runtime,
                    session: &session,
                    mouse,
                    image_support: &image_support,
                    preview: &mut image_renderer,
                    inline: &mut inline_images,
                }
                .run(|| match &payload.decoded {
                    Ok(decoded) if decoded.format == crate::images::ImageSourceFormat::Svg => {
                        image_viewer.open_rendered(&decoded.pixels, &viewer_preference)
                    }
                    _ => image_viewer.open(&payload.bytes, &viewer_preference),
                })?;
                dirty = true;
            }
            if let Some(model) = &mut runtime.model
                && let Some(direction) = model.take_page_turn()
            {
                animation = if event::poll(Duration::ZERO)? {
                    None
                } else {
                    Some(RunningAnimation::new(
                        direction,
                        model.view().width as u16,
                        Instant::now(),
                    ))
                };
                dirty = true;
            }
            if let Some(running) = &animation {
                if running.is_finished(Instant::now()) {
                    animation = None;
                }
                dirty = true;
            }
            if runtime.is_finished() {
                return Ok(());
            }
            if signals.requested.swap(false, Ordering::SeqCst) {
                animation = None;
                runtime.shutdown();
                dirty = true;
            }
            if last_input.elapsed() >= Duration::from_millis(300) {
                runtime.autosave();
            }
            if dirty && last_frame.elapsed() >= FRAME_INTERVAL {
                terminal.draw(|frame| {
                    if !usable_size(screen_size.width, screen_size.height) {
                        render::draw_small_terminal(frame);
                        return;
                    }
                    draw(
                        frame,
                        runtime.model.as_ref(),
                        animation.is_none().then_some(&inline_images),
                    );
                    if let (Some(running), Some(model)) = (&animation, &runtime.model) {
                        let panes = render::page_panes(model, frame.area());
                        running.paint(frame.buffer_mut(), &panes, Instant::now());
                    }
                    image_renderer.draw(frame, runtime.model.as_ref());
                })?;
                last_frame = Instant::now();
                dirty = false;
            }
            let timeout = if dirty {
                FRAME_INTERVAL
                    .saturating_sub(last_frame.elapsed())
                    .min(Duration::from_millis(8))
            } else {
                Duration::from_millis(8)
            };
            if !event::poll(timeout)? {
                continue;
            }
            let batch_started = Instant::now();
            for _ in 0..256 {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        mouse_tracker.cancel();
                        animation = None;
                        dirty = true;
                        if let Some(model) = &mut runtime.model {
                            model.cancel_page_turn();
                        }
                        if let Some(key) = input_key(key) {
                            runtime.handle_key(key);
                            dirty = true;
                            last_input = Instant::now();
                        }
                    }
                    Event::Resize(columns, rows) => {
                        mouse_tracker.cancel();
                        screen_size = ratatui::layout::Size::new(columns, rows);
                        let support = image_support.detect();
                        image_renderer.set_support(support);
                        inline_images.set_support(support);
                        animation = None;
                        if usable_size(columns, rows) {
                            runtime.resize(viewport(columns, rows));
                        } else {
                            image_renderer.clear();
                            inline_images.clear();
                        }
                        dirty = true;
                    }
                    Event::Paste(text) => {
                        mouse_tracker.cancel();
                        animation = None;
                        runtime.handle_paste(&text);
                        dirty = true;
                        last_input = Instant::now();
                    }
                    Event::Mouse(event)
                        if mouse && usable_size(screen_size.width, screen_size.height) =>
                    {
                        if let Some(action) =
                            mouse_tracker.input(event, screen_size, runtime.model.as_ref())
                        {
                            animation = None;
                            runtime.handle_mouse(action);
                            dirty = true;
                            last_input = Instant::now();
                        }
                    }
                    Event::FocusLost => {
                        mouse_tracker.cancel();
                        animation = None;
                        dirty = true;
                        if let Some(model) = &mut runtime.model {
                            model.cancel_page_turn();
                        }
                    }
                    _ => {}
                }
                if runtime.is_finished() {
                    return Ok(());
                }
                if batch_started.elapsed() >= Duration::from_millis(4)
                    || !event::poll(Duration::ZERO)?
                {
                    break;
                }
            }
        }
    })();
    if result.is_err() {
        finish_lost_terminal(&mut runtime);
    }
    result
}

// #NOTE: A failed terminal cannot confirm dialogs, but recent progress still gets a bounded save attempt.
fn finish_lost_terminal(runtime: &mut ReaderRuntime) {
    runtime.shutdown();
    let deadline = Instant::now() + Duration::from_millis(250);
    while !runtime.is_finished() && Instant::now() < deadline {
        if runtime.poll().is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
