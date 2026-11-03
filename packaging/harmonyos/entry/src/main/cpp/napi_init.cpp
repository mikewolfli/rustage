/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
 * SPDX-License-Identifier: MIT
 *
 * The NAPI module the page imports as `libentry.so`.
 *
 * Every function below forwards to a C entry point published by the pinned
 * rust-widgets revision (`include/rw_generated.h`): the module owns argument
 * conversion and nothing else. The bridge rules it follows are the ones the
 * library's `examples/harmony_napi_bridge_flow.md` states:
 *
 *   1. `rw_init`, then `rw_create_*`; keep the returned ids;
 *   2. hand the ArkUI `XComponent` to `rw_harmony_bind_xcomponent` once, and the
 *      mapping to `rw_harmony_bind_node` when a node must address a widget;
 *   3. forward callbacks through `rw_harmony_on_*`;
 *   4. drain `rw_poll_widget_trigger_event` on the application's tick;
 *   5. forward a size change through `rw_report_window_resize`, because ArkUI
 *      reports it to the component, not to the library's backend.
 *
 * The linked `.so` must be built with the `xcomponent` feature, otherwise
 * `rw_harmony_bind_xcomponent` answers `false` and logs the reason.
 */
#include <cstdint>
#include <string>
#include <vector>

#include <ace/xcomponent/native_interface_xcomponent.h>
#include <napi/native_api.h>

#include "rw_generated.h"

namespace {

napi_value Undefined(napi_env env) {
    napi_value value = nullptr;
    napi_get_undefined(env, &value);
    return value;
}

napi_value Boolean(napi_env env, bool value) {
    napi_value out = nullptr;
    napi_get_boolean(env, value, &out);
    return out;
}

napi_value Number(napi_env env, double value) {
    napi_value out = nullptr;
    napi_create_double(env, value, &out);
    return out;
}

bool ReadId(napi_env env, napi_value value, uint64_t* out) {
    double raw = 0.0;
    if (napi_get_value_double(env, value, &raw) != napi_ok) {
        return false;
    }
    // A NaN (a Length that was not a number) and a negative value are both refusals: casting
    // either to an id would address an unrelated widget.
    if (!(raw >= 0.0 && raw <= 9007199254740991.0)) {
        return false;
    }
    *out = static_cast<uint64_t>(raw);
    return true;
}

bool ReadExtent(napi_env env, napi_value value, unsigned int* out) {
    double raw = 0.0;
    if (napi_get_value_double(env, value, &raw) != napi_ok) {
        return false;
    }
    if (!(raw >= 0.0 && raw <= 4294967295.0)) {
        return false;
    }
    *out = static_cast<unsigned int>(raw);
    return true;
}

bool ReadCoordinate(napi_env env, napi_value value, int* out) {
    double raw = 0.0;
    if (napi_get_value_double(env, value, &raw) != napi_ok) {
        return false;
    }
    if (!(raw >= -2147483648.0 && raw <= 2147483647.0)) {
        return false;
    }
    *out = static_cast<int>(raw);
    return true;
}

bool ReadString(napi_env env, napi_value value, std::string* out) {
    size_t length = 0;
    if (napi_get_value_string_utf8(env, value, nullptr, 0, &length) != napi_ok) {
        return false;
    }
    std::vector<char> buffer(length + 1, '\0');
    size_t copied = 0;
    if (napi_get_value_string_utf8(env, value, buffer.data(), buffer.size(), &copied) != napi_ok) {
        return false;
    }
    out->assign(buffer.data(), copied);
    return true;
}

/// The XComponent context ArkTS passes to `onLoad`, unwrapped to the native component.
OH_NativeXComponent* NativeComponent(napi_env env, napi_value context) {
    napi_value exportInstance = nullptr;
    if (napi_get_named_property(env, context, OH_NATIVE_XCOMPONENT_OBJ, &exportInstance) != napi_ok) {
        return nullptr;
    }
    OH_NativeXComponent* component = nullptr;
    if (napi_unwrap(env, exportInstance, reinterpret_cast<void**>(&component)) != napi_ok) {
        return nullptr;
    }
    return component;
}

napi_value Init(napi_env env, napi_callback_info /*info*/) {
    rw_init();
    return Undefined(env);
}

napi_value CreateWindow(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string title;
    unsigned int width = 0;
    unsigned int height = 0;
    if (argc < 3 || !ReadString(env, argv[0], &title) || !ReadExtent(env, argv[1], &width) ||
        !ReadExtent(env, argv[2], &height)) {
        return Number(env, 0.0);
    }
    return Number(env, static_cast<double>(rw_create_window(title.c_str(), 0, 0, width, height)));
}

napi_value CreateButton(napi_env env, napi_callback_info info) {
    size_t argc = 6;
    napi_value argv[6] = {nullptr, nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t windowId = 0;
    std::string text;
    int x = 0;
    int y = 0;
    unsigned int width = 0;
    unsigned int height = 0;
    if (argc < 6 || !ReadId(env, argv[0], &windowId) || !ReadString(env, argv[1], &text) ||
        !ReadCoordinate(env, argv[2], &x) || !ReadCoordinate(env, argv[3], &y) ||
        !ReadExtent(env, argv[4], &width) || !ReadExtent(env, argv[5], &height)) {
        return Number(env, 0.0);
    }
    return Number(env, static_cast<double>(rw_create_button(windowId, text.c_str(), x, y, width, height)));
}

napi_value MountSurface(napi_env env, napi_callback_info info) {
    size_t argc = 6;
    napi_value argv[6] = {nullptr, nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t parentId = 0;
    uint64_t widgetId = 0;
    int x = 0;
    int y = 0;
    unsigned int width = 0;
    unsigned int height = 0;
    if (argc < 6 || !ReadId(env, argv[0], &parentId) || !ReadId(env, argv[1], &widgetId) ||
        !ReadCoordinate(env, argv[2], &x) || !ReadCoordinate(env, argv[3], &y) ||
        !ReadExtent(env, argv[4], &width) || !ReadExtent(env, argv[5], &height)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_mount_surface(parentId, widgetId, x, y, width, height));
}

napi_value BindComponent(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) {
        return Boolean(env, false);
    }
    OH_NativeXComponent* component = NativeComponent(env, argv[0]);
    if (component == nullptr) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_harmony_bind_xcomponent(reinterpret_cast<uintptr_t>(component)));
}

napi_value BindNode(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t nodeHandle = 0;
    uint64_t widgetId = 0;
    if (argc < 2 || !ReadId(env, argv[0], &nodeHandle) || !ReadId(env, argv[1], &widgetId)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_harmony_bind_node(nodeHandle, widgetId));
}

napi_value UnbindNode(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t nodeHandle = 0;
    if (argc < 1 || !ReadId(env, argv[0], &nodeHandle)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_harmony_unbind_node(nodeHandle));
}

napi_value OnWidgetClick(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t widgetId = 0;
    if (argc < 1 || !ReadId(env, argv[0], &widgetId)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_harmony_on_click(widgetId));
}

napi_value OnNodeClick(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t nodeHandle = 0;
    if (argc < 1 || !ReadId(env, argv[0], &nodeHandle)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_harmony_on_node_click(nodeHandle));
}

napi_value ReportResize(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    uint64_t windowId = 0;
    unsigned int width = 0;
    unsigned int height = 0;
    if (argc < 3 || !ReadId(env, argv[0], &windowId) || !ReadExtent(env, argv[1], &width) ||
        !ReadExtent(env, argv[2], &height)) {
        return Boolean(env, false);
    }
    return Boolean(env, rw_report_window_resize(windowId, width, height));
}

napi_value TakeRepaint(napi_env env, napi_callback_info /*info*/) {
    return Number(env, static_cast<double>(rw_take_pending_repaint()));
}

napi_value PollTrigger(napi_env env, napi_callback_info /*info*/) {
    uint64_t widgetId = 0;
    const unsigned int kind = rw_poll_widget_trigger_event(&widgetId);
    napi_value out = nullptr;
    napi_create_object(env, &out);
    napi_set_named_property(env, out, "widgetId", Number(env, static_cast<double>(widgetId)));
    napi_set_named_property(env, out, "kind", Number(env, static_cast<double>(kind)));
    return out;
}

napi_value ClearNodeBindings(napi_env env, napi_callback_info /*info*/) {
    rw_harmony_clear_node_bindings();
    return Undefined(env);
}

napi_value Register(napi_env env, napi_value exports) {
    napi_property_descriptor properties[] = {
        {"init", nullptr, Init, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"createWindow", nullptr, CreateWindow, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"createButton", nullptr, CreateButton, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"mountSurface", nullptr, MountSurface, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"bindComponent", nullptr, BindComponent, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"bindNode", nullptr, BindNode, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"unbindNode", nullptr, UnbindNode, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"onWidgetClick", nullptr, OnWidgetClick, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"onNodeClick", nullptr, OnNodeClick, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"reportResize", nullptr, ReportResize, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"takeRepaint", nullptr, TakeRepaint, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"pollTrigger", nullptr, PollTrigger, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"clearNodeBindings", nullptr, ClearNodeBindings, nullptr, nullptr, nullptr, napi_default,
         nullptr},
    };
    napi_define_properties(env, exports, sizeof(properties) / sizeof(properties[0]), properties);
    return exports;
}

napi_module entryModule = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Register,
    .nm_modname = "entry",
    .nm_priv = nullptr,
    .reserved = {0},
};

} // namespace

extern "C" __attribute__((constructor)) void EntryModuleInit(void) {
    napi_module_register(&entryModule);
}
