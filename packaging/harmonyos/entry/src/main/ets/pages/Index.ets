/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
 * SPDX-License-Identifier: MIT
 *
 * The page that carries the native bridge. It creates the library's window and one
 * button, hands the ArkUI `XComponent` to `rw_harmony_bind_xcomponent`, forwards a
 * click and a resize, and drains the library's trigger queue.
 *
 * The native module (`libentry.so`) is `napi_init.cpp`; its declarations are in
 * `src/main/cpp/types/libentry/index.d.ts`. The library's C entry points are the
 * pinned revision's `include/rw_generated.h` — this page does not spell an ABI of
 * its own (a second header would be a second source of truth).
 */
import nativeUi from 'libentry.so';
import { hilog } from '@kit.PerformanceAnalysisKit';

const DOMAIN: number = 0x0000;
const TAG: string = 'rustage';

interface Trigger {
  widgetId: number;
  kind: number;
}

@Entry
@Component
struct Index {
  private windowId: number = 0;
  private buttonId: number = 0;
  private triggerTimer: number = -1;

  aboutToAppear(): void {
    nativeUi.init();
    this.windowId = nativeUi.createWindow('rustage', 1280, 800);
    if (this.windowId !== 0) {
      this.buttonId = nativeUi.createButton(this.windowId, 'Native button', 24, 24, 240, 48);
      nativeUi.mountSurface(this.windowId, this.buttonId, 24, 88, 240, 48);
    } else {
      hilog.error(DOMAIN, TAG, 'rw_create_window returned 0');
    }
    this.triggerTimer = setInterval(() => {
      const trigger: Trigger = nativeUi.pollTrigger() as Trigger;
      if (trigger.kind !== 0) {
        hilog.info(DOMAIN, TAG, 'widget %{public}d raised kind %{public}d', trigger.widgetId, trigger.kind);
      }
    }, 16);
  }

  aboutToDisappear(): void {
    if (this.triggerTimer !== -1) {
      clearInterval(this.triggerTimer);
      this.triggerTimer = -1;
    }
    nativeUi.clearNodeBindings();
  }

  build() {
    Column() {
      XComponent({
        id: 'rustage_surface',
        type: XComponentType.SURFACE,
        libraryname: 'entry',
      })
        .width('100%')
        .height('70%')
        .onLoad((context: object) => {
          if (!nativeUi.bindComponent(context)) {
            hilog.error(DOMAIN, TAG, 'rw_harmony_bind_xcomponent returned false');
          }
        })
      Button('Forward a click to the native button')
        .margin(12)
        .onClick(() => {
          if (!nativeUi.onWidgetClick(this.buttonId)) {
            hilog.warn(DOMAIN, TAG, 'rw_harmony_on_click returned false for widget %{public}d', this.buttonId);
          }
        })
    }
    .width('100%')
    .height('100%')
    .onAreaChange((_oldValue: Area, newValue: Area) => {
      if (this.windowId !== 0) {
        nativeUi.reportResize(this.windowId, Number(newValue.width), Number(newValue.height));
      }
    })
  }
}
