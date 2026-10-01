#include "frame_graph.h"

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
