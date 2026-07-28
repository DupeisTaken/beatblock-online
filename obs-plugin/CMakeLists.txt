cmake_minimum_required(VERSION 3.28)
project(beatblock-online-obs VERSION 0.2.0 LANGUAGES C)

find_package(libobs REQUIRED)
add_library(beatblock-online-obs MODULE src/plugin.c)
target_link_libraries(beatblock-online-obs PRIVATE OBS::libobs)
target_compile_definitions(beatblock-online-obs PRIVATE UNICODE _UNICODE)
set_target_properties(beatblock-online-obs PROPERTIES
  PREFIX ""
  OUTPUT_NAME "beatblock-online-obs"
)
install(TARGETS beatblock-online-obs
  RUNTIME DESTINATION "obs-plugins/64bit"
  LIBRARY DESTINATION "obs-plugins/64bit"
)
install(FILES data/locale/en-US.ini DESTINATION "data/obs-plugins/beatblock-online-obs/locale")

include(CTest)
if(BUILD_TESTING)
  add_executable(beatblock-online-audio-target-test tests/audio-target.c)
  add_test(NAME beatblock-online-audio-target COMMAND beatblock-online-audio-target-test)
endif()
