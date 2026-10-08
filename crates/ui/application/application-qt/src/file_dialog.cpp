#include "file_dialog.h"

#include <QApplication>
#include <QFileDialog>
#include <QFileInfo>
#include <QIcon>
#include <QPalette>
#include <QQuickStyle>
#include <QWindow>

#ifdef Q_OS_WIN
#include <dwmapi.h>
#include <windows.h>
#endif

#include "cxx-qt-lib/qcoreapplication.h"

namespace shrimply {

std::unique_ptr<QGuiApplication> new_widget_application()
{
  QVector<QByteArray> arguments{ QByteArrayLiteral("shrimply") };
  auto *argument_data = new rust::cxxqtlib1::ApplicationArgsData(arguments);
  auto application = std::make_unique<QApplication>(argument_data->size(),
                                                     argument_data->data());
  argument_data->setParent(application.get());
  // Widget themes such as Kvantum are not Qt Quick Controls styles.
  const auto quick_style = qEnvironmentVariable("QT_QUICK_CONTROLS_STYLE");
#ifdef Q_OS_WIN
  QQuickStyle::setStyle(quick_style.isEmpty() ? QStringLiteral("FluentWinUI3") : quick_style);
#else
  QQuickStyle::setStyle(quick_style.isEmpty() ? QStringLiteral("Fusion") : quick_style);
#endif
  QApplication::setWindowIcon(QIcon(QStringLiteral(
    ":/qt/qml/dev/shrimply/application/shrimply-symbolic.svg")));
  return application;
}

void apply_windows_system_backdrop()
{
#ifdef Q_OS_WIN
  const auto apply_backdrop = [](QWindow *window) {
    if (!window) {
      return;
    }
    const auto hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd) {
      return;
    }

    const BOOL dark_mode =
      QGuiApplication::palette().color(QPalette::Window).lightnessF() < 0.5;
    DwmSetWindowAttribute(hwnd,
                          DWMWA_USE_IMMERSIVE_DARK_MODE,
                          &dark_mode,
                          sizeof(dark_mode));

    const auto palette = QGuiApplication::palette();
    const auto window_color = palette.color(QPalette::Window);
    const COLORREF caption_color =
      RGB(window_color.red(), window_color.green(), window_color.blue());
    DwmSetWindowAttribute(hwnd,
                          DWMWA_CAPTION_COLOR,
                          &caption_color,
                          sizeof(caption_color));

    const auto text_color = palette.color(QPalette::WindowText);
    const COLORREF caption_text_color =
      RGB(text_color.red(), text_color.green(), text_color.blue());
    DwmSetWindowAttribute(hwnd,
                          DWMWA_TEXT_COLOR,
                          &caption_text_color,
                          sizeof(caption_text_color));

    const DWM_SYSTEMBACKDROP_TYPE backdrop = DWMSBT_MAINWINDOW;
    DwmSetWindowAttribute(hwnd,
                          DWMWA_SYSTEMBACKDROP_TYPE,
                          &backdrop,
                          sizeof(backdrop));
  };

  const auto windows = QGuiApplication::topLevelWindows();
  for (auto *window : windows) {
    apply_backdrop(window);
  }
#endif
}

static void prepare_dialog(QFileDialog &dialog,
                           const QString &title,
                           const QString &filter)
{
  dialog.setWindowTitle(title);
  dialog.setNameFilter(filter);
#ifndef Q_OS_WIN
  dialog.setOption(QFileDialog::DontUseNativeDialog);
#endif
  dialog.setWindowModality(Qt::WindowModal);

  dialog.winId();
  if (auto *window = dialog.windowHandle()) {
    window->setTransientParent(QGuiApplication::focusWindow());
  }
}

QUrl open_file_dialog(const QUrl &initial_url,
                      const QString &title,
                      const QString &filter)
{
  QFileDialog dialog;
  const QFileInfo initial_file(initial_url.toLocalFile());
  prepare_dialog(dialog, title, filter);
  if (!initial_file.filePath().isEmpty()) {
    dialog.setDirectory(initial_file.isDir() ? initial_file.filePath()
                                             : initial_file.absolutePath());
    if (!initial_file.isDir()) {
      dialog.selectFile(initial_file.fileName());
    }
  }
  dialog.setAcceptMode(QFileDialog::AcceptOpen);
  dialog.setFileMode(QFileDialog::ExistingFile);

  if (dialog.exec() != QDialog::Accepted || dialog.selectedUrls().isEmpty()) {
    return {};
  }
  return dialog.selectedUrls().constFirst();
}

QUrl save_file_dialog(const QUrl &suggested_url,
                      const QString &title,
                      const QString &filter,
                      const QString &default_suffix)
{
  QFileDialog dialog;
  const QFileInfo suggested_file(suggested_url.toLocalFile());
  prepare_dialog(dialog, title, filter);
  dialog.setDirectory(suggested_file.absolutePath());
  dialog.selectFile(suggested_file.fileName());
  dialog.setAcceptMode(QFileDialog::AcceptSave);
  dialog.setFileMode(QFileDialog::AnyFile);
  dialog.setDefaultSuffix(default_suffix);

  if (dialog.exec() != QDialog::Accepted || dialog.selectedUrls().isEmpty()) {
    return {};
  }
  return dialog.selectedUrls().constFirst();
}

} // namespace shrimply
