#pragma once

#include <memory>

#include <QGuiApplication>
#include <QString>
#include <QUrl>

namespace shrimply {

std::unique_ptr<QGuiApplication> new_widget_application();
void apply_windows_system_backdrop();
QUrl open_file_dialog(const QUrl &initial_url,
                      const QString &title,
                      const QString &filter);
QUrl save_file_dialog(const QUrl &suggested_url,
                      const QString &title,
                      const QString &filter,
                      const QString &default_suffix);

} // namespace shrimply
