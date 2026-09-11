#include "file_dialog.h"

#include <QApplication>
#include <QFileDialog>
#include <QFileInfo>
#include <QQuickStyle>
#include <QWindow>

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
  QQuickStyle::setStyle(quick_style.isEmpty() ? QStringLiteral("Fusion") : quick_style);
  return application;
}

static void prepare_dialog(QFileDialog &dialog,
                           const QString &title,
                           const QString &filter)
{
  dialog.setWindowTitle(title);
  dialog.setNameFilter(filter);
  dialog.setOption(QFileDialog::DontUseNativeDialog);
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
