#pragma once

#include <QMessageBox>

namespace shrimply {
inline void show_startup_error(const QString &heading, const QString &body)
{
  QMessageBox::critical(nullptr, heading, body, QMessageBox::Close);
}
}
