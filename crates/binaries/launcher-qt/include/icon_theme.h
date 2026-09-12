#pragma once

#include <QGuiApplication>
#include <QIcon>
#include <QStyleHints>

namespace shrimply {
inline void set_breeze_icon_fallback()
{
#ifdef Q_OS_WIN
  QIcon::setThemeName(QStringLiteral("shrimply-adwaita"));
  QIcon::setFallbackThemeName(QStringLiteral("shrimply-adwaita"));
#else
  const auto color_scheme = QGuiApplication::styleHints()->colorScheme();
  QIcon::setFallbackThemeName(color_scheme == Qt::ColorScheme::Dark
                                  ? QStringLiteral("breeze-dark")
                                  : QStringLiteral("breeze"));
#endif
}
}
