#include "gpu_surface.h"

#include <QByteArray>
#include <QClipboard>
#include <QColor>
#include <QCursor>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QHoverEvent>
#include <QIcon>
#include <QImage>
#include <QMouseEvent>
#include <QKeyEvent>
#include <QPointer>
#include <QOpenGLFramebufferObject>
#include <QOpenGLFramebufferObjectFormat>
#include <QPalette>
#include <QQuickOpenGLUtils>
#include <QQuickWindow>
#include <QSGRendererInterface>
#include <QDesktopServices>
#include <QVariantMap>
#include <QWheelEvent>
#if defined(Q_OS_LINUX)
#include <QtGui/qguiapplication_platform.h>
#endif
#include <QtQml/qqml.h>

#include <cstddef>
#include <cstdint>

struct ShrimplyPaletteColor {
    float red;
    float green;
    float blue;
    float alpha;
};

struct ShrimplyPlatformPalette {
    ShrimplyPaletteColor window_bg;
    ShrimplyPaletteColor window_fg;
    ShrimplyPaletteColor view_bg;
    ShrimplyPaletteColor view_fg;
    ShrimplyPaletteColor alternate_bg;
    ShrimplyPaletteColor button_bg;
    ShrimplyPaletteColor button_fg;
    ShrimplyPaletteColor border;
    ShrimplyPaletteColor accent_bg;
    ShrimplyPaletteColor accent_fg;
};

constexpr std::size_t PLATFORM_PALETTE_COLOR_COUNT = 10;
static_assert(sizeof(ShrimplyPlatformPalette)
              == sizeof(ShrimplyPaletteColor) * PLATFORM_PALETTE_COLOR_COUNT);

extern "C" void shrimply_qt_set_platform_palette(const ShrimplyPlatformPalette *palette);
extern "C" bool shrimply_qt_render_timeline(std::uint32_t width, std::uint32_t height,
                                             float scale, float red, float green,
                                             float blue, float alpha, bool dark);
extern "C" bool shrimply_qt_render_preview(std::uint32_t width, std::uint32_t height,
                                            float scale, float red, float green,
                                            float blue, float alpha, bool dark,
                                            bool fullscreen);
extern "C" bool shrimply_qt_render_audio_meter(std::uint32_t width, std::uint32_t height,
                                                float scale, bool dark);
extern "C" void shrimply_qt_timeline_pointer_move(float x, float y, bool control, bool shift);
extern "C" std::uint8_t shrimply_qt_timeline_pointer_cursor();
extern "C" void shrimply_qt_timeline_pointer_leave();
extern "C" void shrimply_qt_timeline_pointer_press(std::uint8_t button, float x, float y,
                                                     bool control, bool shift);
extern "C" void shrimply_qt_timeline_pointer_release(std::uint8_t button, float x, float y,
                                                       bool control, bool shift);
extern "C" void shrimply_qt_timeline_scroll(float dx, float dy, bool control, bool shift);
extern "C" bool shrimply_qt_timeline_take_track_add_menu();
extern "C" bool shrimply_qt_timeline_take_error();
extern "C" float shrimply_qt_timeline_track_add_menu_x();
extern "C" float shrimply_qt_timeline_track_add_menu_y();
extern "C" std::size_t shrimply_qt_timeline_track_add_menu_count();
extern "C" std::uint8_t shrimply_qt_timeline_track_add_menu_kind(std::size_t index);
extern "C" std::size_t shrimply_qt_timeline_track_add_menu_label(std::size_t index,
                                                                   std::uint8_t *output,
                                                                   std::size_t capacity);
extern "C" std::size_t shrimply_qt_timeline_track_add_menu_icon(std::size_t index,
                                                                  std::uint8_t *output,
                                                                  std::size_t capacity);
extern "C" bool shrimply_qt_timeline_activate_track_add_menu_item(std::size_t index);
extern "C" std::uint8_t shrimply_qt_timeline_import_track_file(const std::uint8_t *path,
                                                                 std::size_t length);
extern "C" bool shrimply_qt_timeline_confirm_track_remux(bool remux);
extern "C" std::size_t shrimply_qt_timeline_prepare_context_menu(float x, float y);
extern "C" std::size_t shrimply_qt_timeline_context_menu_label(std::size_t index,
                                                                 std::uint8_t *output,
                                                                 std::size_t capacity);
extern "C" std::size_t shrimply_qt_timeline_context_menu_count();
extern "C" std::uint8_t shrimply_qt_timeline_context_menu_kind(std::size_t index);
extern "C" bool shrimply_qt_timeline_context_menu_enabled(std::size_t index);
extern "C" double shrimply_qt_timeline_context_menu_value(std::size_t index);
extern "C" double shrimply_qt_timeline_context_menu_minimum(std::size_t index);
extern "C" double shrimply_qt_timeline_context_menu_maximum(std::size_t index);
extern "C" double shrimply_qt_timeline_context_menu_step(std::size_t index);
extern "C" bool shrimply_qt_timeline_context_menu_mixed(std::size_t index);
extern "C" void shrimply_qt_timeline_set_context_menu_control(std::size_t index, double value);
extern "C" std::uint8_t shrimply_qt_timeline_key(std::uint32_t key, bool shortcut, bool shift);
extern "C" std::uint8_t shrimply_qt_timeline_activate_context_menu_item(std::size_t index);
extern "C" std::int32_t shrimply_qt_timeline_context_frame_width();
extern "C" std::int32_t shrimply_qt_timeline_context_frame_height();
extern "C" std::size_t shrimply_qt_timeline_copy_context_frame(std::uint8_t *output,
                                                                std::size_t capacity);
extern "C" std::size_t shrimply_qt_timeline_context_action_error(std::uint8_t *output,
                                                                  std::size_t capacity);
extern "C" std::size_t shrimply_qt_timeline_context_open_path(std::uint8_t *output,
                                                               std::size_t capacity);
extern "C" std::size_t shrimply_qt_timeline_context_delete_clip_count();
extern "C" void shrimply_qt_timeline_delete_context_folded_track();
extern "C" std::size_t shrimply_qt_timeline_clipboard_marker(std::uint8_t *output,
                                                              std::size_t capacity);
extern "C" void shrimply_qt_timeline_paste_clipboard_text(const std::uint8_t *text,
                                                           std::size_t length);
extern "C" bool shrimply_qt_timeline_magnet();
extern "C" void shrimply_qt_timeline_set_magnet(bool enabled);
extern "C" bool shrimply_qt_timeline_beat_grid();
extern "C" void shrimply_qt_timeline_set_beat_grid(bool enabled);
extern "C" bool shrimply_qt_timeline_cut_enabled();
extern "C" void shrimply_qt_timeline_set_cut_enabled(bool enabled);
extern "C" bool shrimply_qt_timeline_overwrite_mode();
extern "C" bool shrimply_qt_timeline_block_mode();
extern "C" bool shrimply_qt_timeline_new_track_mode();
extern "C" void shrimply_qt_timeline_select_overwrite_mode();
extern "C" void shrimply_qt_timeline_select_block_mode();
extern "C" void shrimply_qt_timeline_select_new_track_mode();
#if defined(Q_OS_WINDOWS)
extern "C" bool shrimply_qt_timeline_begin_pointer_lock(float scale);
#else
extern "C" bool shrimply_qt_timeline_begin_pointer_lock(void *display, void *surface,
                                                          void *seat);
#endif
extern "C" void shrimply_qt_timeline_relative_motion(float x, float y);
extern "C" void shrimply_qt_timeline_end_pointer_lock(bool control, bool shift);
extern "C" void shrimply_qt_preview_pointer_move(float width, float height, float x, float y,
                                                    bool control, bool shift, bool alt);
extern "C" std::uint8_t shrimply_qt_preview_pointer_cursor();
extern "C" void shrimply_qt_preview_pointer_leave();
extern "C" bool shrimply_qt_preview_pointer_press(float width, float height, float x, float y,
                                                     bool control, bool shift, bool alt);
extern "C" void shrimply_qt_preview_pointer_release(float width, float height, float x, float y,
                                                       bool control, bool shift, bool alt);
extern "C" void shrimply_qt_preview_pointer_cancel();
enum PreviewNavigationPhase : std::uint8_t { PreviewPanBegin, PreviewPanEnd, PreviewZoom };
constexpr float PreviewScrollPixelsPerStep = 120.0f;
extern "C" bool shrimply_qt_preview_navigation(float width, float height, float x, float y, std::uint8_t phase, float steps);
extern "C" bool shrimply_qt_preview_guides_visible();
extern "C" void shrimply_qt_preview_set_guides_visible(bool visible);

namespace {
enum class TrackFileImportResult : std::uint8_t {
    Error,
    Started,
    ConfirmRemux,
};

QOpenGLFramebufferObject *make_fbo(const QSize &size) {
    QOpenGLFramebufferObjectFormat format;
    format.setAttachment(QOpenGLFramebufferObject::NoAttachment);
    format.setSamples(0);
    return new QOpenGLFramebufferObject(size, format);
}

bool dark_palette() {
    const QColor window = QGuiApplication::palette().color(QPalette::Window);
    return window.lightnessF() < 0.5;
}

ShrimplyPaletteColor palette_color(const QPalette &palette, QPalette::ColorRole role) {
    const QColor color = palette.color(role);
    return {color.redF(), color.greenF(), color.blueF(), color.alphaF()};
}

ShrimplyPlatformPalette platform_palette() {
    const QPalette palette = QGuiApplication::palette();
    return {
        palette_color(palette, QPalette::Window),
        palette_color(palette, QPalette::WindowText),
        palette_color(palette, QPalette::Base),
        palette_color(palette, QPalette::Text),
        palette_color(palette, QPalette::AlternateBase),
        palette_color(palette, QPalette::Button),
        palette_color(palette, QPalette::ButtonText),
        palette_color(palette, QPalette::Mid),
        palette_color(palette, QPalette::Highlight),
        palette_color(palette, QPalette::HighlightedText),
    };
}

QImage context_frame_image() {
    const int width = shrimply_qt_timeline_context_frame_width();
    const int height = shrimply_qt_timeline_context_frame_height();
    const std::size_t length = shrimply_qt_timeline_copy_context_frame(nullptr, 0);
    if (width <= 0 || height <= 0 || length != static_cast<std::size_t>(width) * height * 4) {
        return {};
    }
    QByteArray pixels(static_cast<qsizetype>(length), Qt::Uninitialized);
    if (shrimply_qt_timeline_copy_context_frame(
            reinterpret_cast<std::uint8_t *>(pixels.data()), length) != length) {
        return {};
    }
    return QImage(reinterpret_cast<const uchar *>(pixels.constData()), width, height,
                  width * 4, QImage::Format_RGBA8888).copy();
}

QString context_action_error() {
    const std::size_t length = shrimply_qt_timeline_context_action_error(nullptr, 0);
    QByteArray message(static_cast<qsizetype>(length + 1), Qt::Uninitialized);
    shrimply_qt_timeline_context_action_error(
        reinterpret_cast<std::uint8_t *>(message.data()), static_cast<std::size_t>(message.size()));
    return QString::fromUtf8(message.constData());
}

QString context_open_path() {
    const std::size_t length = shrimply_qt_timeline_context_open_path(nullptr, 0);
    QByteArray path(static_cast<qsizetype>(length + 1), Qt::Uninitialized);
    shrimply_qt_timeline_context_open_path(
        reinterpret_cast<std::uint8_t *>(path.data()), static_cast<std::size_t>(path.size()));
    return QString::fromUtf8(path.constData());
}

QString timeline_clipboard_marker() {
    const std::size_t length = shrimply_qt_timeline_clipboard_marker(nullptr, 0);
    QByteArray marker(static_cast<qsizetype>(length + 1), Qt::Uninitialized);
    shrimply_qt_timeline_clipboard_marker(
        reinterpret_cast<std::uint8_t *>(marker.data()), static_cast<std::size_t>(marker.size()));
    return QString::fromUtf8(marker.constData());
}

class TimelineRenderer final : public QQuickFramebufferObject::Renderer {
public:
    QOpenGLFramebufferObject *createFramebufferObject(const QSize &size) override {
        return make_fbo(size);
    }

    void synchronize(QQuickFramebufferObject *item) override {
        surface_ = static_cast<shrimply::TimelineSurface *>(item);
        scale_ = item->window() ? item->window()->effectiveDevicePixelRatio() : 1.0f;
    }

    void render() override {
        const QSize size = framebufferObject()->size();
        const ShrimplyPlatformPalette palette = platform_palette();
        shrimply_qt_set_platform_palette(&palette);
        if (!shrimply_qt_render_timeline(
                static_cast<std::uint32_t>(size.width()),
                static_cast<std::uint32_t>(size.height()), scale_,
                palette.accent_bg.red, palette.accent_bg.green,
                palette.accent_bg.blue, palette.accent_bg.alpha,
                dark_palette())) {
            qFatal("Shrimply could not render the timeline with OpenGL");
        }
        if (shrimply_qt_timeline_take_track_add_menu()) {
            const QPointer<shrimply::TimelineSurface> surface = surface_;
            QMetaObject::invokeMethod(
                surface_,
                [surface]() {
                    if (surface) {
                        surface->presentTrackAddMenu();
                    }
                },
                Qt::QueuedConnection);
        }
        if (shrimply_qt_timeline_take_error()) {
            const QPointer<shrimply::TimelineSurface> surface = surface_;
            QMetaObject::invokeMethod(
                surface_,
                [surface]() {
                    if (surface) {
                        surface->presentTimelineError();
                    }
                },
                Qt::QueuedConnection);
        }
        QQuickOpenGLUtils::resetOpenGLState();
        update();
    }

private:
    QPointer<shrimply::TimelineSurface> surface_;
    float scale_ = 1.0f;
};

class PreviewRenderer final : public QQuickFramebufferObject::Renderer {
public:
    QOpenGLFramebufferObject *createFramebufferObject(const QSize &size) override {
        return make_fbo(size);
    }

    void synchronize(QQuickFramebufferObject *item) override {
        scale_ = item->window() ? item->window()->effectiveDevicePixelRatio() : 1.0f;
        fullscreen_ = static_cast<shrimply::PreviewSurface *>(item)->fullscreenPreview();
    }

    void render() override {
        const QSize size = framebufferObject()->size();
        const ShrimplyPlatformPalette palette = platform_palette();
        shrimply_qt_set_platform_palette(&palette);
        if (!shrimply_qt_render_preview(
                static_cast<std::uint32_t>(size.width()),
                static_cast<std::uint32_t>(size.height()), scale_,
                palette.window_bg.red, palette.window_bg.green, palette.window_bg.blue,
                palette.window_bg.alpha, dark_palette(), fullscreen_)) {
            qFatal("Shrimply could not render the preview with OpenGL");
        }
        QQuickOpenGLUtils::resetOpenGLState();
        update();
    }

private:
    float scale_ = 1.0f;
    bool fullscreen_ = false;
};

class AudioMeterRenderer final : public QQuickFramebufferObject::Renderer {
public:
    QOpenGLFramebufferObject *createFramebufferObject(const QSize &size) override {
        return make_fbo(size);
    }

    void synchronize(QQuickFramebufferObject *item) override {
        scale_ = item->window() ? item->window()->effectiveDevicePixelRatio() : 1.0f;
    }

    void render() override {
        const QSize size = framebufferObject()->size();
        const ShrimplyPlatformPalette palette = platform_palette();
        shrimply_qt_set_platform_palette(&palette);
        if (!shrimply_qt_render_audio_meter(
                static_cast<std::uint32_t>(size.width()),
                static_cast<std::uint32_t>(size.height()), scale_,
                dark_palette())) {
            qFatal("Shrimply could not render the audio meter with OpenGL");
        }
        QQuickOpenGLUtils::resetOpenGLState();
        update();
    }

private:
    float scale_ = 1.0f;
};

void modifiers(const QInputEvent *event, bool &control, bool &shift) {
    control = event->modifiers().testFlag(Qt::ControlModifier);
    shift = event->modifiers().testFlag(Qt::ShiftModifier);
}

std::uint8_t pointer_button(Qt::MouseButton button) {
    return button == Qt::MiddleButton ? 1 : 0;
}

void update_timeline_cursor(QQuickItem *item) {
    switch (shrimply_qt_timeline_pointer_cursor()) {
    case 1:
    case 2:
    case 3:
        item->setCursor(QCursor(Qt::SizeHorCursor));
        break;
    case 4:
        item->setCursor(QCursor(Qt::CrossCursor));
        break;
    default:
        item->unsetCursor();
    }
}

void update_preview_cursor(QQuickItem *item) {
    switch (shrimply_qt_preview_pointer_cursor()) {
    case 1:
        item->setCursor(QCursor(Qt::SizeHorCursor));
        break;
    case 2:
        item->setCursor(QCursor(Qt::SizeVerCursor));
        break;
    case 3:
        item->setCursor(QCursor(Qt::PointingHandCursor));
        break;
    case 4:
        item->setCursor(QCursor(Qt::CrossCursor));
        break;
    case 5:
        item->setCursor(QCursor(Qt::SizeAllCursor));
        break;
    case 6:
        item->setCursor(QCursor(Qt::OpenHandCursor));
        break;
    case 7:
        item->setCursor(QCursor(Qt::ClosedHandCursor));
        break;
    case 8:
        item->setCursor(QCursor(Qt::IBeamCursor));
        break;
    case 9:
        item->setCursor(QCursor(Qt::SizeHorCursor));
        break;
    case 10:
        item->setCursor(QCursor(Qt::SizeVerCursor));
        break;
    case 11:
        item->setCursor(QCursor(Qt::SizeFDiagCursor));
        break;
    case 12:
        item->setCursor(QCursor(Qt::SizeBDiagCursor));
        break;
    case 13:
        item->setCursor(QCursor(Qt::BlankCursor));
        break;
    default:
        item->unsetCursor();
    }
}

} // namespace

namespace shrimply {

void force_opengl() {
    qputenv("QSG_RENDER_LOOP", "basic");
    QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
}

void configure_icons() {
#ifdef Q_OS_WIN
    QIcon::setThemeName(QStringLiteral("shrimply-adwaita"));
    QIcon::setFallbackThemeName(QStringLiteral("shrimply-adwaita"));
#else
    const QString theme = dark_palette() ? QStringLiteral("breeze-dark") : QStringLiteral("breeze");
    QIcon::setThemeName(theme);
    QIcon::setFallbackThemeName(theme);
#endif
}

QString fixed_font_family() {
    return QFontDatabase::systemFont(QFontDatabase::FixedFont).family();
}

void register_gpu_surfaces() {
    qmlRegisterType<TimelineSurface>("dev.shrimply.editor", 1, 0, "TimelineSurface");
    qmlRegisterType<PreviewSurface>("dev.shrimply.editor", 1, 0, "PreviewSurface");
    qmlRegisterType<AudioMeterSurface>("dev.shrimply.editor", 1, 0, "AudioMeterSurface");
}

TimelineSurface::TimelineSurface(QQuickItem *parent) : QQuickFramebufferObject(parent) {
    setMirrorVertically(true);
    setAcceptedMouseButtons(Qt::LeftButton | Qt::MiddleButton | Qt::RightButton);
    setAcceptHoverEvents(true);
}

QQuickFramebufferObject::Renderer *TimelineSurface::createRenderer() const {
    return new TimelineRenderer();
}

bool TimelineSurface::magnetEnabled() const {
    return shrimply_qt_timeline_magnet();
}

void TimelineSurface::setMagnetEnabled(bool enabled) {
    if (magnetEnabled() == enabled) {
        return;
    }
    shrimply_qt_timeline_set_magnet(enabled);
    emit magnetEnabledChanged();
    update();
}

bool TimelineSurface::beatGridEnabled() const {
    return shrimply_qt_timeline_beat_grid();
}

void TimelineSurface::setBeatGridEnabled(bool enabled) {
    if (beatGridEnabled() == enabled) {
        return;
    }
    shrimply_qt_timeline_set_beat_grid(enabled);
    emit beatGridEnabledChanged();
    update();
}

bool TimelineSurface::cutEnabled() const {
    return shrimply_qt_timeline_cut_enabled();
}

void TimelineSurface::setCutEnabled(bool enabled) {
    if (cutEnabled() == enabled) {
        return;
    }
    shrimply_qt_timeline_set_cut_enabled(enabled);
    emit cursorToolChanged();
    update();
}

bool TimelineSurface::overwriteMode() const {
    return shrimply_qt_timeline_overwrite_mode();
}

bool TimelineSurface::blockMode() const {
    return shrimply_qt_timeline_block_mode();
}

bool TimelineSurface::newTrackMode() const {
    return shrimply_qt_timeline_new_track_mode();
}

void TimelineSurface::selectOverwriteMode() {
    if (overwriteMode()) {
        return;
    }
    shrimply_qt_timeline_select_overwrite_mode();
    emit dragCollisionModeChanged();
    update();
}

void TimelineSurface::selectBlockMode() {
    if (blockMode()) {
        return;
    }
    shrimply_qt_timeline_select_block_mode();
    emit dragCollisionModeChanged();
    update();
}

void TimelineSurface::selectNewTrackMode() {
    if (newTrackMode()) {
        return;
    }
    shrimply_qt_timeline_select_new_track_mode();
    emit dragCollisionModeChanged();
    update();
}

QVariantList TimelineSurface::contextMenuItems() const {
    QVariantList items;
    const std::size_t count = shrimply_qt_timeline_context_menu_count();
    for (std::size_t index = 0; index < count; ++index) {
        std::uint8_t buffer[256];
        shrimply_qt_timeline_context_menu_label(index, buffer, sizeof(buffer));
        QVariantMap item;
        item.insert(QStringLiteral("index"), static_cast<qulonglong>(index));
        item.insert(QStringLiteral("kind"), shrimply_qt_timeline_context_menu_kind(index));
        item.insert(QStringLiteral("label"),
                    QString::fromUtf8(reinterpret_cast<const char *>(buffer)));
        item.insert(QStringLiteral("enabled"),
                    shrimply_qt_timeline_context_menu_enabled(index));
        item.insert(QStringLiteral("value"), shrimply_qt_timeline_context_menu_value(index));
        item.insert(QStringLiteral("minimum"), shrimply_qt_timeline_context_menu_minimum(index));
        item.insert(QStringLiteral("maximum"), shrimply_qt_timeline_context_menu_maximum(index));
        item.insert(QStringLiteral("step"), shrimply_qt_timeline_context_menu_step(index));
        item.insert(QStringLiteral("mixed"), shrimply_qt_timeline_context_menu_mixed(index));
        items.append(item);
    }
    return items;
}

QVariantList TimelineSurface::trackAddMenuItems() const {
    QVariantList items;
    const std::size_t count = shrimply_qt_timeline_track_add_menu_count();
    for (std::size_t index = 0; index < count; ++index) {
        QVariantMap item;
        item.insert(QStringLiteral("index"), static_cast<qulonglong>(index));
        item.insert(QStringLiteral("kind"), shrimply_qt_timeline_track_add_menu_kind(index));
        std::uint8_t label[256];
        shrimply_qt_timeline_track_add_menu_label(index, label, sizeof(label));
        item.insert(QStringLiteral("label"),
                    QString::fromUtf8(reinterpret_cast<const char *>(label)));
        std::uint8_t icon[256];
        shrimply_qt_timeline_track_add_menu_icon(index, icon, sizeof(icon));
        item.insert(QStringLiteral("icon"),
                    QString::fromUtf8(reinterpret_cast<const char *>(icon)));
        items.append(item);
    }
    return items;
}

void TimelineSurface::presentTrackAddMenu() {
    emit trackAddMenuItemsChanged();
    emit trackAddMenuRequested(shrimply_qt_timeline_track_add_menu_x(),
                               shrimply_qt_timeline_track_add_menu_y());
}

void TimelineSurface::presentTimelineError() {
    emit contextActionFailed(context_action_error());
}

void TimelineSurface::activateTrackAddMenuItem(int index) {
    if (index < 0 || index >= trackAddMenuItems().size()) {
        return;
    }
    if (shrimply_qt_timeline_activate_track_add_menu_item(static_cast<std::size_t>(index))) {
        emit trackImportRequested();
    }
    update();
}

void TimelineSurface::importTrackFile(const QUrl &url) {
    const QByteArray path = url.toLocalFile().toUtf8();
    if (path.isEmpty()) {
        emit contextActionFailed(context_action_error());
        return;
    }
    const auto result = static_cast<TrackFileImportResult>(
        shrimply_qt_timeline_import_track_file(
            reinterpret_cast<const std::uint8_t *>(path.constData()),
            static_cast<std::size_t>(path.size())));
    if (result == TrackFileImportResult::Error) {
        emit contextActionFailed(context_action_error());
    } else if (result == TrackFileImportResult::ConfirmRemux) {
        emit trackRemuxRequested();
    }
    update();
}

void TimelineSurface::confirmTrackRemux(bool remux) {
    if (!shrimply_qt_timeline_confirm_track_remux(remux)) {
        emit contextActionFailed(context_action_error());
    }
    update();
}

void TimelineSurface::setContextMenuControl(int index, qreal value) {
    if (index < 0 || index >= contextMenuItems().size()) {
        return;
    }
    shrimply_qt_timeline_set_context_menu_control(static_cast<std::size_t>(index), value);
    update();
}

void TimelineSurface::activateContextMenuItem(int index) {
    if (index < 0 || index >= contextMenuItems().size()) {
        return;
    }
    const std::uint8_t result =
        shrimply_qt_timeline_activate_context_menu_item(static_cast<std::size_t>(index));
    handleActionResult(result);
    emit contextMenuItemsChanged();
}

void TimelineSurface::keyPressEvent(QKeyEvent *event) {
    if (event->modifiers().testFlag(Qt::AltModifier) || event->modifiers().testFlag(Qt::MetaModifier)) {
        event->ignore();
        return;
    }
    std::uint32_t key = event->key();
    switch (event->key()) {
    case Qt::Key_Escape: key = 0x1b; break;
    case Qt::Key_Delete: key = 0x7f; break;
    case Qt::Key_Backspace: key = 0x08; break;
    default: break;
    }
    const auto result = shrimply_qt_timeline_key(key,
        event->modifiers().testFlag(Qt::ControlModifier), event->modifiers().testFlag(Qt::ShiftModifier));
    constexpr std::uint8_t unhandled_key = UINT8_MAX;
    if (result == unhandled_key) {
        event->ignore();
        return;
    }
    handleActionResult(result);
    event->accept();
}

void TimelineSurface::handleActionResult(std::uint8_t result) {
    if (result == 1) {
        const QImage image = context_frame_image();
        if (image.isNull()) {
            emit contextActionFailed(QStringLiteral("Could not copy the selected frame."));
        } else {
            QGuiApplication::clipboard()->setImage(image);
        }
    } else if (result == 2) {
        emit saveFrameRequested();
    } else if (result == 3) {
        emit contextActionFailed(context_action_error());
    } else if (result == 4) {
        const QString path = context_open_path();
        if (path.isEmpty() || !QDesktopServices::openUrl(QUrl::fromLocalFile(path))) {
            emit contextActionFailed(QStringLiteral("Could not open the containing folder."));
        }
    } else if (result == 5) {
        emit deleteTrackRequested(
            static_cast<int>(shrimply_qt_timeline_context_delete_clip_count()));
    } else if (result == 6) {
        QGuiApplication::clipboard()->setText(timeline_clipboard_marker());
    } else if (result == 7) {
        const QByteArray text = QGuiApplication::clipboard()->text().toUtf8();
        shrimply_qt_timeline_paste_clipboard_text(
            reinterpret_cast<const std::uint8_t *>(text.constData()),
            static_cast<std::size_t>(text.size()));
    }
    emit contextMenuItemsChanged();
    update();
}

void TimelineSurface::deleteContextFoldedTrack() {
    shrimply_qt_timeline_delete_context_folded_track();
    update();
}

void TimelineSurface::saveContextFrame(const QUrl &url) {
    QImage image = context_frame_image();
    QString path = url.toLocalFile();
    if (image.isNull() || path.isEmpty()) {
        emit contextActionFailed(QStringLiteral("Could not save the selected frame."));
        return;
    }
    if (QFileInfo(path).suffix().compare(QStringLiteral("png"), Qt::CaseInsensitive) != 0) {
        path.append(QStringLiteral(".png"));
    }
    if (!image.save(path, "PNG")) {
        emit contextActionFailed(QStringLiteral("Could not save the selected frame."));
    }
}

void TimelineSurface::hoverMoveEvent(QHoverEvent *event) {
    if (middle_mouse_grabbed_) {
        event->accept();
        return;
    }
    bool control;
    bool shift;
    modifiers(event, control, shift);
    shrimply_qt_timeline_pointer_move(event->position().x(), event->position().y(), control, shift);
    update_timeline_cursor(this);
    update();
}

void TimelineSurface::hoverLeaveEvent(QHoverEvent *event) {
    Q_UNUSED(event);
    shrimply_qt_timeline_pointer_leave();
    unsetCursor();
    update();
}

void TimelineSurface::mousePressEvent(QMouseEvent *event) {
    forceActiveFocus(Qt::MouseFocusReason);
    if (event->button() == Qt::RightButton) {
        if (shrimply_qt_timeline_prepare_context_menu(event->position().x(),
                                                       event->position().y()) > 0) {
            emit contextMenuItemsChanged();
            emit contextMenuRequested(event->position().x(), event->position().y());
            update();
        }
        event->accept();
        return;
    }
    bool control;
    bool shift;
    modifiers(event, control, shift);
    shrimply_qt_timeline_pointer_press(pointer_button(event->button()), event->position().x(),
                                       event->position().y(), control, shift);
    if (event->button() == Qt::MiddleButton) {
#if defined(Q_OS_WINDOWS)
        const float scale = window() ? static_cast<float>(window()->effectiveDevicePixelRatio())
                                     : 1.0f;
        if (shrimply_qt_timeline_begin_pointer_lock(scale)) {
            middle_cursor_origin_ = QCursor::pos();
            middle_cursor_center_ = mapToGlobal(QPointF(width() / 2.0, height() / 2.0)).toPoint();
            QCursor::setPos(middle_cursor_center_);
#else
        auto *wayland = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
        void *surface = window() ? reinterpret_cast<void *>(window()->winId()) : nullptr;
        if (wayland && shrimply_qt_timeline_begin_pointer_lock(
                           wayland->display(), surface, wayland->seat())) {
#endif
            middle_mouse_grabbed_ = true;
            setKeepMouseGrab(true);
            setCursor(QCursor(Qt::BlankCursor));
        }
    }
    event->accept();
    update();
}

void TimelineSurface::mouseMoveEvent(QMouseEvent *event) {
    if (middle_mouse_grabbed_) {
#if defined(Q_OS_WINDOWS)
        const QPoint delta = event->globalPosition().toPoint() - middle_cursor_center_;
        if (!delta.isNull()) {
            shrimply_qt_timeline_relative_motion(delta.x(), delta.y());
            QCursor::setPos(middle_cursor_center_);
        }
#endif
        event->accept();
        update();
        return;
    }
    bool control;
    bool shift;
    modifiers(event, control, shift);
    shrimply_qt_timeline_pointer_move(event->position().x(), event->position().y(), control, shift);
    update_timeline_cursor(this);
    event->accept();
    update();
}

void TimelineSurface::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() == Qt::RightButton) {
        event->accept();
        return;
    }
    bool control;
    bool shift;
    modifiers(event, control, shift);
    if (event->button() == Qt::MiddleButton && middle_mouse_grabbed_) {
        shrimply_qt_timeline_end_pointer_lock(control, shift);
        middle_mouse_grabbed_ = false;
        setKeepMouseGrab(false);
        unsetCursor();
#if defined(Q_OS_WINDOWS)
        QCursor::setPos(middle_cursor_origin_);
#endif
    } else {
        shrimply_qt_timeline_pointer_release(pointer_button(event->button()),
                                             event->position().x(), event->position().y(),
                                             control, shift);
    }
    event->accept();
    update();
}

void TimelineSurface::mouseUngrabEvent() {
    if (!middle_mouse_grabbed_) {
        return;
    }
    shrimply_qt_timeline_end_pointer_lock(false, false);
    middle_mouse_grabbed_ = false;
    setKeepMouseGrab(false);
    unsetCursor();
#if defined(Q_OS_WINDOWS)
    QCursor::setPos(middle_cursor_origin_);
#endif
    update();
}

void TimelineSurface::wheelEvent(QWheelEvent *event) {
    bool control;
    bool shift;
    modifiers(event, control, shift);
    const QPointF delta = event->pixelDelta().isNull()
                              ? QPointF(event->angleDelta()) / 120.0
                              : QPointF(event->pixelDelta()) / 120.0;
    shrimply_qt_timeline_scroll(-delta.x(), -delta.y(), control, shift);
    event->accept();
    update();
}

PreviewSurface::PreviewSurface(QQuickItem *parent) : QQuickFramebufferObject(parent) {
    setMirrorVertically(true);
    setAcceptedMouseButtons(Qt::LeftButton | Qt::MiddleButton);
    setAcceptHoverEvents(true);
}

QQuickFramebufferObject::Renderer *PreviewSurface::createRenderer() const {
    return new PreviewRenderer();
}

bool PreviewSurface::guidesVisible() const {
    return shrimply_qt_preview_guides_visible();
}

void PreviewSurface::setGuidesVisible(bool visible) {
    if (guidesVisible() == visible) {
        return;
    }
    shrimply_qt_preview_set_guides_visible(visible);
    emit guidesVisibleChanged();
    update();
}

bool PreviewSurface::fullscreenPreview() const {
    return fullscreen_preview_;
}

void PreviewSurface::setFullscreenPreview(bool fullscreen) {
    if (fullscreen_preview_ == fullscreen) {
        return;
    }
    fullscreen_preview_ = fullscreen;
    emit fullscreenPreviewChanged();
    update();
}

void PreviewSurface::hoverMoveEvent(QHoverEvent *event) {
    const auto modifiers = event->modifiers();
    shrimply_qt_preview_pointer_move(width(), height(), event->position().x(),
                                     event->position().y(),
                                     modifiers.testFlag(Qt::ControlModifier),
                                     modifiers.testFlag(Qt::ShiftModifier),
                                     modifiers.testFlag(Qt::AltModifier));
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::hoverLeaveEvent(QHoverEvent *event) {
    shrimply_qt_preview_pointer_leave();
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::mousePressEvent(QMouseEvent *event) {
    if (event->button() == Qt::MiddleButton) {
        if (!shrimply_qt_preview_navigation(width(), height(), event->position().x(), event->position().y(), PreviewPanBegin, 0.0f)) {
            event->ignore();
            return;
        }
        forceActiveFocus(Qt::MouseFocusReason);
        setKeepMouseGrab(true);
        update_preview_cursor(this);
        event->accept();
        update();
        return;
    }
    const auto modifiers = event->modifiers();
    if (event->button() != Qt::LeftButton ||
        !shrimply_qt_preview_pointer_press(width(), height(), event->position().x(),
                                           event->position().y(),
                                           modifiers.testFlag(Qt::ControlModifier),
                                           modifiers.testFlag(Qt::ShiftModifier),
                                           modifiers.testFlag(Qt::AltModifier))) {
        event->ignore();
        return;
    }
    forceActiveFocus(Qt::MouseFocusReason);
    setKeepMouseGrab(true);
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::mouseMoveEvent(QMouseEvent *event) {
    const auto modifiers = event->modifiers();
    shrimply_qt_preview_pointer_move(width(), height(), event->position().x(),
                                     event->position().y(),
                                     modifiers.testFlag(Qt::ControlModifier),
                                     modifiers.testFlag(Qt::ShiftModifier),
                                     modifiers.testFlag(Qt::AltModifier));
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() == Qt::MiddleButton) {
        shrimply_qt_preview_navigation(width(), height(), event->position().x(), event->position().y(), PreviewPanEnd, 0.0f);
        setKeepMouseGrab(false);
        update_preview_cursor(this);
        event->accept();
        update();
        return;
    }
    if (event->button() != Qt::LeftButton) {
        event->ignore();
        return;
    }
    const auto modifiers = event->modifiers();
    shrimply_qt_preview_pointer_release(width(), height(), event->position().x(),
                                        event->position().y(),
                                        modifiers.testFlag(Qt::ControlModifier),
                                        modifiers.testFlag(Qt::ShiftModifier),
                                        modifiers.testFlag(Qt::AltModifier));
    setKeepMouseGrab(false);
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::wheelEvent(QWheelEvent *event) {
    const float delta = event->pixelDelta().isNull() ? event->angleDelta().y() : event->pixelDelta().y();
    if (!shrimply_qt_preview_navigation(width(), height(), event->position().x(), event->position().y(), PreviewZoom, -delta / PreviewScrollPixelsPerStep)) {
        event->ignore();
        return;
    }
    update_preview_cursor(this);
    event->accept();
    update();
}

void PreviewSurface::mouseUngrabEvent() {
    shrimply_qt_preview_pointer_cancel();
    setKeepMouseGrab(false);
    update_preview_cursor(this);
    update();
}

AudioMeterSurface::AudioMeterSurface(QQuickItem *parent) : QQuickFramebufferObject(parent) {
    setMirrorVertically(true);
}

QQuickFramebufferObject::Renderer *AudioMeterSurface::createRenderer() const {
    return new AudioMeterRenderer();
}

} // namespace shrimply
