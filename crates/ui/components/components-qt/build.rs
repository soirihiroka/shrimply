use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    unsafe {
        CxxQtBuilder::new_qml_module(QmlModule::new("dev.shrimply.components").qml_files([
            "qml/AdaptiveMenu.qml",
            "qml/AdaptiveComboBox.qml",
            "qml/MenuInput.qml",
            "qml/ButtonSelector.qml",
            "qml/ColorPicker.qml",
            "qml/Checkerboard.qml",
            "qml/CodeEditor.qml",
            "qml/CompletionPopup.qml",
            "qml/CollapsibleSection.qml",
            "qml/ColorSliderThumb.qml",
            "qml/ControlRow.qml",
            "qml/Dropdown.qml",
            "qml/ExpressionEditor.qml",
            "qml/FrameGraph.qml",
            "qml/FontFamilyList.qml",
            "qml/InspectorCard.qml",
            "qml/InspectorGraphProperty.qml",
            "qml/InspectorPairGraphProperty.qml",
            "qml/InspectorTripleGraphProperty.qml",
            "qml/InspectorTextGraphProperty.qml",
            "qml/InspectorProperty.qml",
            "qml/InspectorPropertyRow.qml",
            "qml/LivePerformance.qml",
            "qml/MultilineTextInput.qml",
            "qml/ModifierMenuButton.qml",
            "qml/NumberPicker.qml",
            "qml/Number2Picker.qml",
            "qml/Number3Picker.qml",
            "qml/PaletteSwatch.qml",
            "qml/PlaybackShortcuts.qml",
            "qml/ProgressButton.qml",
            "qml/ProjectSettingsSelector.qml",
            "qml/ReadOnlyField.qml",
            "qml/SearchMenu.qml",
            "qml/Selector.qml",
            "qml/SingleLineTextInput.qml",
            "qml/SplitButton.qml",
            "qml/SwitchRow.qml",
            "qml/Tabs.qml",
            "qml/TextContextMenu.qml",
            "qml/TransparentColorPreview.qml",
            "qml/TypoUnderline.qml",
        ]))
        .files(["src/backend.rs", "src/frame_graph.rs"])
        .cpp_files([
            "include/color_settings.h",
            "src/color_settings.cpp",
            "include/drag_input.h",
            "src/drag_input.cpp",
            "include/frame_graph.h",
            "src/frame_graph.cpp",
        ])
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("OpenGL")
        .cc_builder(|build| {
            build.include("include");
            build.flag_if_supported("-Wno-sfinae-incomplete");
        })
        .build();
    }
}
