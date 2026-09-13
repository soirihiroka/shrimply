#pragma once

#include <QQuickItem>
#include <QQuickTextDocument>
#include <QPointer>
#include <QPoint>
#include <QSyntaxHighlighter>
#include <QTimer>

#include <memory>

namespace shrimply {

void register_drag_input();

class DragInput : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(qreal threshold READ threshold WRITE setThreshold)

public:
    explicit DragInput(QQuickItem *parent = nullptr);
    qreal threshold() const;
    void setThreshold(qreal threshold);

signals:
    void dragStarted();
    void dragged(qreal offset);
    void dragFinished();
    void clicked();

protected:
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseUngrabEvent() override;

private:
    void finish();
    bool beginPointerLock();

    QTimer poll_timer_;
    qreal threshold_ = 2.0;
    qreal start_x_ = 0.0;
    qreal accumulated_x_ = 0.0;
    QPoint cursor_center_;
    QPoint cursor_origin_;
    bool pressed_ = false;
    bool moved_ = false;
    bool lock_attempted_ = false;
    bool locked_ = false;
};

class TypoSyntaxHighlighter;
class CodeSyntaxHighlighter;

class TypoHighlighter : public QObject {
    Q_OBJECT
    Q_PROPERTY(QQuickTextDocument *document READ document WRITE setDocument NOTIFY documentChanged)
    Q_PROPERTY(QString ranges READ ranges WRITE setRanges NOTIFY rangesChanged)

public:
    explicit TypoHighlighter(QObject *parent = nullptr);
    ~TypoHighlighter() override;
    QQuickTextDocument *document() const;
    void setDocument(QQuickTextDocument *document);
    QString ranges() const;
    void setRanges(const QString &ranges);

signals:
    void documentChanged();
    void rangesChanged();

private:
    void rebuild();

    QPointer<QQuickTextDocument> document_;
    QString ranges_;
    std::unique_ptr<TypoSyntaxHighlighter> highlighter_;
};

class CodeHighlighter : public QObject {
    Q_OBJECT
    Q_PROPERTY(QQuickTextDocument *document READ document WRITE setDocument NOTIFY documentChanged)
    Q_PROPERTY(int diagnosticLine READ diagnosticLine WRITE setDiagnosticLine NOTIFY diagnosticChanged)
    Q_PROPERTY(int diagnosticColumn READ diagnosticColumn WRITE setDiagnosticColumn NOTIFY diagnosticChanged)

public:
    explicit CodeHighlighter(QObject *parent = nullptr);
    ~CodeHighlighter() override;
    QQuickTextDocument *document() const;
    void setDocument(QQuickTextDocument *document);
    int diagnosticLine() const;
    void setDiagnosticLine(int line);
    int diagnosticColumn() const;
    void setDiagnosticColumn(int column);

signals:
    void documentChanged();
    void diagnosticChanged();

private:
    void rebuild();

    QPointer<QQuickTextDocument> document_;
    std::unique_ptr<CodeSyntaxHighlighter> highlighter_;
    int diagnostic_line_ = -1;
    int diagnostic_column_ = -1;
};

} // namespace shrimply
