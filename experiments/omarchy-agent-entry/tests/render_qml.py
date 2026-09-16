"""Optional real QtQuick layout test. Requires PySide6, not needed by the plugin."""

import importlib.util
import sys
from pathlib import Path

from PySide6.QtCore import QObject, QPoint, Qt, QTimer, QUrl
from PySide6.QtGui import QGuiApplication
from PySide6.QtQuick import QQuickView
from PySide6.QtTest import QTest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("entry", ROOT / "agent_entry.py")
entry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entry)
app = QGuiApplication(sys.argv)
view = QQuickView()
view.setResizeMode(QQuickView.SizeRootObjectToView)
view.setSource(QUrl.fromLocalFile(str(ROOT / "DashboardContent.qml")))
if view.status() == QQuickView.Error:
    raise SystemExit("\n".join(e.toString() for e in view.errors()))
root = view.rootObject()
items = entry.catalog()
for item in items:
    item["available"] = False
    item["status"] = "尚未发现启动入口"
root.setProperty("agents", items)
root.setProperty("project", "/home/user/Projects/anolisa")
root.setProperty("message", "QtQuick 排版预览：未连接 Omarchy 或运行任何 Agent。")
view.setTitle("ANOLISA Agent Entry — layout QA")
view.resize(1160, 740)
view.show()
output = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/anolisa-agent-entry-preview.png")
actions = []
root.actionRequested.connect(
    lambda value: actions.append(value.toVariant() if hasattr(value, "toVariant") else value)
)


def click(name: str) -> None:
    item = root.findChild(QObject, name)
    if item is None:
        raise RuntimeError("Missing UI item: " + name)
    point = item.mapToScene(item.boundingRect().center())
    QTest.mouseClick(view, Qt.LeftButton, Qt.NoModifier, QPoint(round(point.x()), round(point.y())))
    QTest.qWait(80)


def capture() -> None:
    click("openTerminal")
    assert actions[-1] == [
        "launch",
        "terminal",
        "--project",
        "/home/user/Projects/anolisa",
    ], actions
    if not view.grabWindow().save(str(output)):
        raise RuntimeError("Failed to capture " + str(output))
    print(output)
    click("addEntry")
    dialog = root.findChild(QObject, "registerDialog")
    assert dialog.property("visible"), "Registration dialog did not open"
    view.grabWindow().save(str(output.with_name(output.stem + "-register.png")))
    QTest.keyClick(view, Qt.Key_Escape)
    QTest.qWait(80)
    assert not dialog.property("visible"), "Escape did not close dialog"
    view.resize(800, 680)
    QTest.qWait(150)
    view.grabWindow().save(str(output.with_name(output.stem + "-narrow.png")))
    print("PASS: QtQuick load, terminal action signal, registration dialog, Escape, resize")
    app.quit()


QTimer.singleShot(1200, capture)
raise SystemExit(app.exec())
