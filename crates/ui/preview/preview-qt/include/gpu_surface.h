#pragma once

#include <QQuickFramebufferObject>
#include <QString>
#include <QUrl>
#include <QVariantList>

namespace shrimply {

void force_opengl();
void configure_icons();
QString fixed_font_family();
void register_gpu_surfaces();

class TimelineSurface : public QQuickFramebufferObject {
    Q_OBJECT
    Q_PROPERTY(bool magnetEnabled READ magnetEnabled WRITE setMagnetEnabled NOTIFY magnetEnabledChanged)
    Q_PROPERTY(bool beatGridEnabled READ beatGridEnabled WRITE setBeatGridEnabled NOTIFY beatGridEnabledChanged)
    Q_PROPERTY(bool cutEnabled READ cutEnabled WRITE setCutEnabled NOTIFY cursorToolChanged)
    Q_PROPERTY(bool overwriteMode READ overwriteMode NOTIFY dragCollisionModeChanged)
    Q_PROPERTY(bool blockMode READ blockMode NOTIFY dragCollisionModeChanged)
    Q_PROPERTY(bool newTrackMode READ newTrackMode NOTIFY dragCollisionModeChanged)
    Q_PROPERTY(QVariantList contextMenuItems READ contextMenuItems NOTIFY contextMenuItemsChanged)
    Q_PROPERTY(QVariantList trackAddMenuItems READ trackAddMenuItems NOTIFY trackAddMenuItemsChanged)

public:
    explicit TimelineSurface(QQuickItem *parent = nullptr);
    Renderer *createRenderer() const override;
    bool magnetEnabled() const;
    void setMagnetEnabled(bool enabled);
    bool beatGridEnabled() const;
    void setBeatGridEnabled(bool enabled);
    bool cutEnabled() const;
    void setCutEnabled(bool enabled);
    bool overwriteMode() const;
    bool blockMode() const;
    bool newTrackMode() const;
    Q_INVOKABLE void selectOverwriteMode();
    Q_INVOKABLE void selectBlockMode();
    Q_INVOKABLE void selectNewTrackMode();
    QVariantList contextMenuItems() const;
    QVariantList trackAddMenuItems() const;
    Q_INVOKABLE void activateContextMenuItem(int index);
    Q_INVOKABLE void activateTrackAddMenuItem(int index);
    Q_INVOKABLE void importTrackFile(const QUrl &url);
    Q_INVOKABLE void setContextMenuControl(int index, qreal value);
    Q_INVOKABLE void saveContextFrame(const QUrl &url);
    Q_INVOKABLE void deleteContextFoldedTrack();

signals:
    void magnetEnabledChanged();
    void beatGridEnabledChanged();
    void cursorToolChanged();
    void dragCollisionModeChanged();
    void contextMenuItemsChanged();
    void contextMenuRequested(qreal x, qreal y);
    void trackAddMenuItemsChanged();
    void trackAddMenuRequested(qreal x, qreal y);
    void trackImportRequested();
    void saveFrameRequested();
    void contextActionFailed(const QString &message);
    void deleteTrackRequested(int clipCount);

public slots:
    void presentTrackAddMenu();
    void presentTimelineError();

protected:
    void hoverMoveEvent(QHoverEvent *event) override;
    void hoverLeaveEvent(QHoverEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseUngrabEvent() override;
    void wheelEvent(QWheelEvent *event) override;

private:
    bool middle_mouse_grabbed_ = false;
};

class PreviewSurface : public QQuickFramebufferObject {
    Q_OBJECT
    Q_PROPERTY(bool guidesVisible READ guidesVisible WRITE setGuidesVisible NOTIFY guidesVisibleChanged)
    Q_PROPERTY(bool fullscreenPreview READ fullscreenPreview WRITE setFullscreenPreview NOTIFY fullscreenPreviewChanged)

public:
    explicit PreviewSurface(QQuickItem *parent = nullptr);
    Renderer *createRenderer() const override;
    bool guidesVisible() const;
    void setGuidesVisible(bool visible);
    bool fullscreenPreview() const;
    void setFullscreenPreview(bool fullscreen);

signals:
    void guidesVisibleChanged();
    void fullscreenPreviewChanged();

protected:
    void hoverMoveEvent(QHoverEvent *event) override;
    void hoverLeaveEvent(QHoverEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseUngrabEvent() override;
    void wheelEvent(QWheelEvent *event) override;

private:
    bool fullscreen_preview_ = false;
};

class AudioMeterSurface : public QQuickFramebufferObject {
    Q_OBJECT

public:
    explicit AudioMeterSurface(QQuickItem *parent = nullptr);
    Renderer *createRenderer() const override;
};

} // namespace shrimply
