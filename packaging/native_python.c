// PyO3 shares the renderer's abi3 build. PyConfig needs the headers of
// the actual embedded interpreter, selected with PYO3_PYTHON.
#include <Python.h>

#define CHECK(call) do { PyStatus s = (call); if (PyStatus_Exception(s)) Py_ExitStatusException(s); } while (0)

static void add_path(PyConfig *config, const char *suffix) {
    size_t count = wcslen(config->home) + strlen(suffix) + 2;
    wchar_t *path = PyMem_RawCalloc(count, sizeof(*path));
    if (!path) Py_FatalError("allocating bundle path");
    swprintf(path, count, L"%ls/%hs", config->home, suffix);
    CHECK(PyWideStringList_Append(&config->module_search_paths, path));
    PyMem_RawFree(path);
}

void built_initialize(const char *root, const char *exe) {
    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.site_import = 0;
    config.write_bytecode = 0;
    config.module_search_paths_set = 1;
    CHECK(PyConfig_SetBytesString(&config, &config.home, root));
    CHECK(PyConfig_SetBytesString(&config, &config.executable, exe));
    CHECK(PyWideStringList_Append(&config.module_search_paths, config.home));
    add_path(&config, "base_library.zip");
    add_path(&config, "python" Py_STRINGIFY(PY_MAJOR_VERSION) "." Py_STRINGIFY(PY_MINOR_VERSION) "/lib-dynload");
    CHECK(Py_InitializeFromConfig(&config));
    PyConfig_Clear(&config);
    PyEval_SaveThread();
}
