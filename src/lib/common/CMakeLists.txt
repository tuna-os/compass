cmake_minimum_required(VERSION 3.16)
set(PROJECT_NAME vicinae-common)
project(${PROJECT_NAME})
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(BUILD_TESTS "Build the test suite" OFF)

add_library(${PROJECT_NAME} STATIC src/common.cpp)
add_library(vicinae::common ALIAS ${PROJECT_NAME})

target_include_directories(${PROJECT_NAME} PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include PRIVATE src)
target_include_directories(${PROJECT_NAME} INTERFACE ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_compile_definitions(${PROJECT_NAME} PRIVATE VICINAE_LIBEXECDIR="${VICINAE_LIBEXEC_DIR}" VICINAE_LIBEXEC_PATH="${VICINAE_LIBEXEC_PATH}")
target_compile_features(${PROJECT_NAME} PUBLIC cxx_std_23)

if (BUILD_TESTS)
	set(TEST_TARGET ${PROJECT_NAME}-tests)
	find_package(Catch2 3 REQUIRED)
	add_executable(${TEST_TARGET} tests/main.cpp)
	target_link_libraries(${TEST_TARGET} PRIVATE Catch2::Catch2WithMain ${PROJECT_NAME})
	target_compile_features(${TEST_TARGET} PRIVATE cxx_std_23)
endif()
