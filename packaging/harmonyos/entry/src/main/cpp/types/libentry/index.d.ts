/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
 * SPDX-License-Identifier: MIT
 *
 * The native module's surface. Every function forwards to the pinned revision's C
 * ABI (`include/rw_generated.h`); this declaration file names the few entry points
 * the page uses, not a second copy of the whole header.
 */
export const init: () => void;
export const createWindow: (title: string, width: number, height: number) => number;
export const createButton:
  (windowId: number, text: string, x: number, y: number, width: number, height: number) => number;
export const mountSurface:
  (parentId: number, widgetId: number, x: number, y: number, width: number, height: number) => boolean;
export const bindComponent: (component: object) => boolean;
export const bindNode: (nodeHandle: number, widgetId: number) => boolean;
export const unbindNode: (nodeHandle: number) => boolean;
export const onWidgetClick: (widgetId: number) => boolean;
export const onNodeClick: (nodeHandle: number) => boolean;
export const reportResize: (windowId: number, width: number, height: number) => boolean;
export const takeRepaint: () => number;
export const pollTrigger: () => Trigger;
export const clearNodeBindings: () => void;

export interface Trigger {
  widgetId: number;
  kind: number;
}
