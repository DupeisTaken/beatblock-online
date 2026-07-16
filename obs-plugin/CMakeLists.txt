cmake_minimum_required(VERSION 3.28)
project(beatblock-together-obs VERSION 0.2.0 LANGUAGES C)

find_package(libobs REQUIRED)
add_library(beatblock-together-obs MODULE src/plugin.c)
target_link_libraries(beatblock-together-obs PRIVATE OBS::libobs)
target_compile_definitions(beatblock-together-obs PRIVATE UNICODE _UNICODE)
set_target_properties(beatblock-together-obs PROPERTIES
  PREFIX ""
  OUTPUT_NAME "beatblock-together-obs"
)
install(TARGETS beatblock-together-obs
  RUNTIME DESTINATION "obs-plugins/64bit"
  LIBRARY DESTINATION "obs-plugins/64bit"
)
install(FILES data/locale/en-US.ini DESTINATION "data/obs-plugins/beatblock-together-obs/locale")

