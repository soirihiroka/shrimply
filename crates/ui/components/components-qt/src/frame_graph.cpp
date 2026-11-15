#include "frame_graph.h"
#include "shrimply-components-qt/src/frame_graph.cxxqt.h"
#include <QTouchEvent>
#include <vector>
#include <chrono>

#include <QGuiApplication>
#include <QOpenGLFramebufferObject>
#include <QOpenGLFramebufferObjectFormat>
#include <QPalette>
#include <QQuickOpenGLUtils>
#include <QQuickWindow>
#include <QSGRendererInterface>

#include <cstdint>

extern "C" void *shrimply_qt_frame_graph_renderer_new(const void *graph);
extern "C" void shrimply_qt_frame_graph_renderer_free(void *renderer);
extern "C" int shrimply_qt_frame_graph_render(
    void *renderer, std::uint32_t width, std::uint32_t height, float scale,
    float red, float green, float blue, float alpha, bool dark);

namespace {

bool dark_palette() {
    return QGuiApplication::palette().color(QPalette::Window).lightnessF() < 0.5;
}

class FrameGraphRenderer final : public QQuickFramebufferObject::Renderer {
public:
    explicit FrameGraphRenderer(const void *graph)
        : renderer_(shrimply_qt_frame_graph_renderer_new(graph)) {
        if (!renderer_) {
            qFatal("Shrimply could not create the frame graph renderer");
        }
    }

    ~FrameGraphRenderer() override {
        shrimply_qt_frame_graph_renderer_free(renderer_);
    }

    QOpenGLFramebufferObject *createFramebufferObject(const QSize &size) override {
        QOpenGLFramebufferObjectFormat format;
        format.setAttachment(QOpenGLFramebufferObject::NoAttachment);
        format.setSamples(0);
        return new QOpenGLFramebufferObject(size, format);
    }

    void synchronize(QQuickFramebufferObject *item) override {
        scale_ = item->window() ? item->window()->effectiveDevicePixelRatio() : 1.0;
    }

    void render() override {
        const QSize size = framebufferObject()->size();
        const QColor background = QGuiApplication::palette().color(QPalette::Base);
        const int render_state = shrimply_qt_frame_graph_render(
            renderer_, static_cast<std::uint32_t>(size.width()),
            static_cast<std::uint32_t>(size.height()), static_cast<float>(scale_),
            background.redF(), background.greenF(), background.blueF(),
            background.alphaF(), dark_palette());
        if (render_state < 0) {
            qFatal("Shrimply could not render the frame graph with OpenGL");
        }
        QQuickOpenGLUtils::resetOpenGLState();
        if (render_state > 0) {
            update();
        }
    }

private:
    void *renderer_ = nullptr;
    qreal scale_ = 1.0;
};

} // namespace

namespace shrimply {

FrameGraphItemBase::FrameGraphItemBase(QObject *parent)
    : QQuickFramebufferObject(qobject_cast<QQuickItem *>(parent)) {
    setActiveFocusOnTab(true);
    setAcceptTouchEvents(true);
    touch_timer_.setTimerType(Qt::PreciseTimer);
    constexpr int input_poll_hz = 60;
    touch_timer_.setInterval(std::chrono::milliseconds{1000} / input_poll_hz);
    connect(&touch_timer_, &QTimer::timeout, this, [this] {
        if (!frameGraphPollTouch()) touch_timer_.stop();
    });
    const auto cancel_inactive = [this] {
        if (!isEnabled() || !isVisible()) {
            touch_timer_.stop();
            ungrabTouchPoints();
            frameGraphCancelTouch();
        }
    };
    connect(this, &QQuickItem::enabledChanged, this, cancel_inactive);
    connect(this, &QQuickItem::visibleChanged, this, cancel_inactive);
}

FrameGraphItemBase::~FrameGraphItemBase() {
    // Derived Rust state is already destroyed when base teardown emits item signals.
    disconnect(this, nullptr, this, nullptr);
}

void FrameGraphItemBase::touchEvent(QTouchEvent *event) {
    if (event->pointingDevice()->type() != QInputDevice::DeviceType::TouchScreen) {
        event->ignore();
        return;
    }
    if (event->type() == QEvent::TouchCancel) {
        frameGraphCancelTouch();
        touch_timer_.stop();
    } else {
        if (event->type() == QEvent::TouchBegin) {
            // A popup can consume the previous release; Qt's new sequence cancels it.
            frameGraphCancelTouch();
            forceActiveFocus(Qt::MouseFocusReason);
        }
        std::vector<FrameGraphContact> contacts;
        contacts.reserve(event->points().size());
        for (const auto &point : event->points()) {
            const std::uint8_t phase = point.state() == QEventPoint::State::Pressed ? 0
                : point.state() == QEventPoint::State::Released ? 2 : 1;
            contacts.push_back({static_cast<std::uint64_t>(point.id()),
                static_cast<float>(point.position().x()),
                static_cast<float>(point.position().y()), phase});
        }
        if (frameGraphTouch({contacts.data(), contacts.size()})) {
            if (!touch_timer_.isActive()) touch_timer_.start();
        } else touch_timer_.stop();
    }
    event->accept();
}

void FrameGraphItemBase::touchUngrabEvent() {
    touch_timer_.stop();
    frameGraphCancelTouch();
}

QQuickFramebufferObject::Renderer *FrameGraphItemBase::createRenderer() const {
    const std::uintptr_t graph = frameGraphHandle();
    if (graph == 0) {
        qFatal("Shrimply frame graph has no Rust state");
    }
    return new FrameGraphRenderer(reinterpret_cast<const void *>(graph));
}

void force_component_opengl() {
    qputenv("QSG_RENDER_LOOP", "basic");
    QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
}

} // namespace shrimply
