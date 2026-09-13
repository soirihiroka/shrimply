#include "drag_input.h"

#include <QCursor>
#include <QGuiApplication>
#include <QMouseEvent>
#include <QPalette>
#include <QQuickWindow>
#include <QSet>
#include <QTextCharFormat>
#include <QTextDocument>
#if defined(Q_OS_LINUX)
#include <QtGui/qguiapplication_platform.h>
#endif
#include <QtQml/qqml.h>

#include <algorithm>
#include <cmath>

#if defined(Q_OS_LINUX)
extern "C" bool shrimply_qt_number_begin_pointer_lock(void *display, void *surface,
                                                        void *seat);
extern "C" bool shrimply_qt_number_poll_pointer_lock(double *delta_x,
                                                       double *delta_y);
extern "C" void shrimply_qt_number_end_pointer_lock();
#endif

namespace shrimply {

class TypoSyntaxHighlighter final : public QSyntaxHighlighter {
public:
    explicit TypoSyntaxHighlighter(QTextDocument *document)
        : QSyntaxHighlighter(static_cast<QObject *>(nullptr)) {
        setDocument(document);
    }

    void setRanges(const QString &ranges) {
        ranges_.clear();
        for (const QString &range : ranges.split(',', Qt::SkipEmptyParts)) {
            const qsizetype separator = range.indexOf(':');
            if (separator < 0) {
                continue;
            }
            bool start_ok = false;
            bool length_ok = false;
            const int start = range.first(separator).toInt(&start_ok);
            const int length = range.sliced(separator + 1).toInt(&length_ok);
            if (start_ok && length_ok && start >= 0 && length > 0) {
                ranges_.append({start, length});
            }
        }
        rehighlight();
    }

protected:
    void highlightBlock(const QString &) override {
        QTextCharFormat format;
        format.setUnderlineStyle(QTextCharFormat::SpellCheckUnderline);
        format.setUnderlineColor(Qt::red);
        const int block_start = currentBlock().position();
        const int block_end = block_start + currentBlock().length();
        for (const auto &[start, length] : ranges_) {
            const int end = start + length;
            const int visible_start = std::max(start, block_start);
            const int visible_end = std::min(end, block_end);
            if (visible_start < visible_end) {
                setFormat(visible_start - block_start, visible_end - visible_start, format);
            }
        }
    }

private:
    QList<QPair<int, int>> ranges_;
};

class CodeSyntaxHighlighter final : public QSyntaxHighlighter {
public:
    explicit CodeSyntaxHighlighter(QTextDocument *document)
        : QSyntaxHighlighter(static_cast<QObject *>(nullptr)) {
        setDocument(document);
    }

    void setDiagnostic(int line, int column) {
        diagnostic_line_ = line;
        diagnostic_column_ = column;
        rehighlight();
    }

protected:
    void highlightBlock(const QString &text) override {
        const QPalette palette = qGuiApp->palette();
        QTextCharFormat comment;
        comment.setForeground(palette.color(QPalette::PlaceholderText));
        QTextCharFormat string;
        string.setForeground(palette.color(QPalette::LinkVisited));
        QTextCharFormat keyword;
        keyword.setForeground(palette.color(QPalette::Link));
        keyword.setFontWeight(QFont::DemiBold);
        QTextCharFormat function;
        function.setForeground(palette.color(QPalette::Highlight));
        QTextCharFormat variable;
        variable.setForeground(palette.color(QPalette::Highlight));
        QTextCharFormat number;
        number.setForeground(palette.color(QPalette::LinkVisited));

        qsizetype offset = 0;
        if (previousBlockState() == 1) {
            const qsizetype end = text.indexOf("*/");
            if (end < 0) {
                setFormat(0, text.size(), comment);
                setCurrentBlockState(1);
                return;
            }
            setFormat(0, end + 2, comment);
            offset = end + 2;
        }

        while (offset < text.size()) {
            if (text.sliced(offset).startsWith("//")) {
                setFormat(offset, text.size() - offset, comment);
                break;
            }
            if (text.sliced(offset).startsWith("/*")) {
                const qsizetype end = text.indexOf("*/", offset + 2);
                if (end < 0) {
                    setFormat(offset, text.size() - offset, comment);
                    setCurrentBlockState(1);
                    break;
                }
                setFormat(offset, end + 2 - offset, comment);
                offset = end + 2;
                continue;
            }

            const QChar character = text.at(offset);
            if (character == '"' || character == '\'' || character == '`') {
                const QChar quote = character;
                qsizetype end = offset + 1;
                bool escaped = false;
                while (end < text.size()) {
                    const QChar current = text.at(end++);
                    if (!escaped && current == quote) {
                        break;
                    }
                    escaped = !escaped && current == '\\';
                    if (current != '\\') {
                        escaped = false;
                    }
                }
                setFormat(offset, end - offset, string);
                offset = end;
                continue;
            }

            if (character.isDigit()
                && (offset == 0
                    || (!text.at(offset - 1).isLetterOrNumber()
                        && text.at(offset - 1) != '_'))) {
                qsizetype end = offset + 1;
                while (end < text.size()
                       && (text.at(end).isDigit() || text.at(end) == '.')) {
                    ++end;
                }
                if (end < text.size() && (text.at(end) == 'e' || text.at(end) == 'E')) {
                    ++end;
                    if (end < text.size() && (text.at(end) == '+' || text.at(end) == '-')) {
                        ++end;
                    }
                    while (end < text.size() && text.at(end).isDigit()) {
                        ++end;
                    }
                }
                setFormat(offset, end - offset, number);
                offset = end;
                continue;
            }

            if (character.isLetter() || character == '_') {
                qsizetype end = offset + 1;
                while (end < text.size()
                       && (text.at(end).isLetterOrNumber() || text.at(end) == '_')) {
                    ++end;
                }
                const QString word = text.sliced(offset, end - offset);
                if (keywords().contains(word)) {
                    setFormat(offset, end - offset, keyword);
                } else if (functions().contains(word)) {
                    setFormat(offset, end - offset, function);
                } else if (variables().contains(word)) {
                    setFormat(offset, end - offset, variable);
                }
                offset = end;
                continue;
            }
            ++offset;
        }

        if (diagnostic_line_ >= 0
            && (diagnostic_line_ == 0
                || diagnostic_line_ == currentBlock().blockNumber() + 1)) {
            const qsizetype start = diagnostic_line_ == 0
                                       ? 0
                                       : std::clamp<qsizetype>(diagnostic_column_ - 1, 0,
                                                               std::max<qsizetype>(0, text.size() - 1));
            const qsizetype length = diagnostic_line_ == 0
                                         ? text.size()
                                         : std::min<qsizetype>(1, text.size());
            if (length > 0) {
                QTextCharFormat diagnostic = format(start);
                diagnostic.setUnderlineStyle(QTextCharFormat::SpellCheckUnderline);
                diagnostic.setUnderlineColor(QColor(255, 115, 107));
                setFormat(start, length, diagnostic);
            }
        }
    }

private:
    int diagnostic_line_ = -1;
    int diagnostic_column_ = -1;

    static const QSet<QString> &keywords() {
        static const QSet<QString> words = {
            "break", "const", "continue", "else", "export", "false", "fn",
            "for", "if", "in", "let", "loop", "return", "true", "while",
        };
        return words;
    }

    static const QSet<QString> &functions() {
        static const QSet<QString> words = {
            "Fraction", "abs", "clamp", "cos", "int", "lerp", "gray", "graya",
            "hsv", "hsva", "oklab", "oklaba", "pow", "random", "rgb", "rgba",
            "shake", "sin", "sqrt", "tan", "vol",
        };
        return words;
    }

    static const QSet<QString> &variables() {
        static const QSet<QString> words = {
            "canvas_height", "canvas_width", "duration", "fps", "local_t",
            "media_height", "media_width", "seed", "source_height", "source_width",
            "time", "value", "t", "a", "b", "g", "r", "x", "y", "z",
        };
        return words;
    }
};

void register_drag_input() {
    qmlRegisterType<DragInput>("dev.shrimply.components", 1, 0, "DragInput");
    qmlRegisterType<TypoHighlighter>("dev.shrimply.components", 1, 0, "TypoHighlighter");
    qmlRegisterType<CodeHighlighter>("dev.shrimply.components.native", 1, 0,
                                     "CodeHighlighter");
}

DragInput::DragInput(QQuickItem *parent) : QQuickItem(parent) {
    setAcceptedMouseButtons(Qt::LeftButton);
    setCursor(QCursor(Qt::SizeHorCursor));
    poll_timer_.setInterval(8);
    connect(&poll_timer_, &QTimer::timeout, this, [this]() {
#if defined(Q_OS_WINDOWS)
        if (locked_) {
            const QPoint delta = QCursor::pos() - cursor_center_;
            if (!delta.isNull()) {
                accumulated_x_ += delta.x();
                emit dragged(accumulated_x_);
                QCursor::setPos(cursor_center_);
            }
        }
#else
        double delta_x = 0.0;
        double delta_y = 0.0;
        if (locked_ && shrimply_qt_number_poll_pointer_lock(&delta_x, &delta_y)) {
            Q_UNUSED(delta_y);
            accumulated_x_ += delta_x;
            emit dragged(accumulated_x_);
        }
#endif
    });
}

qreal DragInput::threshold() const {
    return threshold_;
}

void DragInput::setThreshold(qreal threshold) {
    threshold_ = std::max<qreal>(0.0, threshold);
}

void DragInput::mousePressEvent(QMouseEvent *event) {
    start_x_ = event->position().x();
    accumulated_x_ = 0.0;
    pressed_ = true;
    moved_ = false;
    lock_attempted_ = false;
    emit dragStarted();
    event->accept();
}

void DragInput::mouseMoveEvent(QMouseEvent *event) {
    if (!pressed_ || locked_) {
        event->accept();
        return;
    }
    const qreal offset = event->position().x() - start_x_;
    if (!moved_ && std::abs(offset) < threshold_) {
        event->accept();
        return;
    }
    moved_ = true;
    accumulated_x_ = offset;
    emit dragged(accumulated_x_);
    if (!lock_attempted_) {
        lock_attempted_ = true;
        locked_ = beginPointerLock();
        if (locked_) {
            setKeepMouseGrab(true);
            setCursor(QCursor(Qt::BlankCursor));
            poll_timer_.start();
        }
    }
    event->accept();
}

void DragInput::mouseReleaseEvent(QMouseEvent *event) {
    finish();
    event->accept();
}

void DragInput::mouseUngrabEvent() {
    if (pressed_) {
        finish();
    }
}

void DragInput::finish() {
    pressed_ = false;
    poll_timer_.stop();
    if (locked_) {
#if defined(Q_OS_WINDOWS)
        QCursor::setPos(cursor_origin_);
#else
        shrimply_qt_number_end_pointer_lock();
#endif
        locked_ = false;
        setKeepMouseGrab(false);
        setCursor(QCursor(Qt::SizeHorCursor));
    }
    if (moved_) {
        emit dragFinished();
    } else {
        emit clicked();
    }
}

bool DragInput::beginPointerLock() {
#if defined(Q_OS_WINDOWS)
    if (!window()) {
        return false;
    }
    cursor_origin_ = QCursor::pos();
    cursor_center_ = mapToGlobal(QPointF(width() / 2.0, height() / 2.0)).toPoint();
    QCursor::setPos(cursor_center_);
    return true;
#else
    auto *wayland = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    void *surface = window() ? reinterpret_cast<void *>(window()->winId()) : nullptr;
    return wayland && shrimply_qt_number_begin_pointer_lock(
                          wayland->display(), surface, wayland->seat());
#endif
}

TypoHighlighter::TypoHighlighter(QObject *parent) : QObject(parent) {}

TypoHighlighter::~TypoHighlighter() = default;

QQuickTextDocument *TypoHighlighter::document() const {
    return document_.data();
}

void TypoHighlighter::setDocument(QQuickTextDocument *document) {
    if (document_ == document) {
        return;
    }
    document_ = document;
    rebuild();
    emit documentChanged();
}

QString TypoHighlighter::ranges() const {
    return ranges_;
}

void TypoHighlighter::setRanges(const QString &ranges) {
    if (ranges_ == ranges) {
        return;
    }
    ranges_ = ranges;
    if (highlighter_) {
        highlighter_->setRanges(ranges_);
    }
    emit rangesChanged();
}

void TypoHighlighter::rebuild() {
    highlighter_.reset();
    if (document_ && document_->textDocument()) {
        highlighter_ = std::make_unique<TypoSyntaxHighlighter>(document_->textDocument());
        highlighter_->setRanges(ranges_);
    }
}

CodeHighlighter::CodeHighlighter(QObject *parent) : QObject(parent) {}

CodeHighlighter::~CodeHighlighter() = default;

QQuickTextDocument *CodeHighlighter::document() const {
    return document_.data();
}

void CodeHighlighter::setDocument(QQuickTextDocument *document) {
    if (document_ == document) {
        return;
    }
    document_ = document;
    rebuild();
    emit documentChanged();
}

int CodeHighlighter::diagnosticLine() const { return diagnostic_line_; }

void CodeHighlighter::setDiagnosticLine(int line) {
    if (diagnostic_line_ == line) {
        return;
    }
    diagnostic_line_ = line;
    if (highlighter_) {
        highlighter_->setDiagnostic(diagnostic_line_, diagnostic_column_);
    }
    emit diagnosticChanged();
}

int CodeHighlighter::diagnosticColumn() const { return diagnostic_column_; }

void CodeHighlighter::setDiagnosticColumn(int column) {
    if (diagnostic_column_ == column) {
        return;
    }
    diagnostic_column_ = column;
    if (highlighter_) {
        highlighter_->setDiagnostic(diagnostic_line_, diagnostic_column_);
    }
    emit diagnosticChanged();
}

void CodeHighlighter::rebuild() {
    highlighter_.reset();
    if (document_ && document_->textDocument()) {
        highlighter_ = std::make_unique<CodeSyntaxHighlighter>(document_->textDocument());
        highlighter_->setDiagnostic(diagnostic_line_, diagnostic_column_);
    }
}

} // namespace shrimply
