# A muted, offline test host for the installed plugins, on the JUCE in ~/Developer/Plugins.
#   cmake -S vst/check -B vst/check/build -DCMAKE_BUILD_TYPE=Release && cmake --build vst/check/build -j8
#   vst/check/build/vstcheck_artefacts/Release/vstcheck "Clip Tape"
cmake_minimum_required(VERSION 3.22)
project(vstcheck VERSION 1.0.0)
set(CMAKE_CXX_STANDARD 17)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../../../Plugins/JUCE juce)
juce_add_gui_app(vstcheck PRODUCT_NAME "vstcheck")
target_sources(vstcheck PRIVATE Main.cpp)
target_compile_definitions(vstcheck PRIVATE JUCE_PLUGINHOST_VST3=1 JUCE_MODAL_LOOPS_PERMITTED=1 JUCE_PLUGINHOST_AU=0 JUCE_WEB_BROWSER=0 JUCE_USE_CURL=0)
target_link_libraries(vstcheck PRIVATE juce::juce_audio_processors juce::juce_audio_utils juce::juce_recommended_config_flags)
