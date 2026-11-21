#pragma once

#include <QQuickFramebufferObject>
#include <cstdint>
#include <QTimer>
#include "rust/cxx.h"

struct FrameGraphContact;

namespace shrimply {

class FrameGraphItemBase : public QQuickFramebufferObject {
    Q_OBJECT

public:
    explicit FrameGraphItemBase(QObject *parent = nullptr);
    ~FrameGraphItemBase() override;
    Renderer *createRenderer() const override;
    virtual std::uintptr_t frameGraphHandle() const = 0;
    virtual bool frameGraphTouch(rust::Slice<const FrameGraphContact> contacts) = 0;
    virtual bool frameGraphPollTouch() = 0;
    virtual void frameGraphCancelTouch() = 0;

protected:
    void touchEvent(QTouchEvent *event) override;
    void touchUngrabEvent() override;

private:
    QTimer touch_timer_;
};

void force_component_opengl();

} // namespace shrimply
