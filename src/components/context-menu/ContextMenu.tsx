import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";

const OPEN_EVENT = "atelier:context-menu-open";
const CLOSE_EVENT = "atelier:context-menu-close";
const VIEWPORT_GUTTER = 8;

export type ContextMenuTriggerEvent = MouseEvent | ReactMouseEvent<Element>;

export type ContextMenuActionItem = {
  type?: "item";
  id: string;
  label: string;
  icon?: ReactNode;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect?: () => void | Promise<void>;
};

export type ContextMenuSeparatorItem = {
  type: "separator";
  id?: string;
};

export type ContextMenuItem = ContextMenuActionItem | ContextMenuSeparatorItem;
export type ContextMenuFactory<TContext> = (context: TContext) => ContextMenuItem[];

export type ContextMenuDefinition<TContext> = {
  id: string;
  buildItems: ContextMenuFactory<TContext>;
};

type ContextMenuOpenDetail = {
  key: number;
  x: number;
  y: number;
  menuId?: string;
  items: ContextMenuItem[];
};

type OpenMenuState = ContextMenuOpenDetail;

let nextMenuKey = 0;
const registry = new Map<string, ContextMenuFactory<unknown>>();

export function defineContextMenu<TContext>(
  id: string,
  buildItems: ContextMenuFactory<TContext>,
): ContextMenuDefinition<TContext> {
  return { id, buildItems };
}

export function registerContextMenu<TContext>(
  definition: ContextMenuDefinition<TContext>,
) {
  registry.set(definition.id, definition.buildItems as ContextMenuFactory<unknown>);
  return () => {
    if (registry.get(definition.id) === definition.buildItems) {
      registry.delete(definition.id);
    }
  };
}

export function unregisterContextMenu(id: string) {
  registry.delete(id);
}

export function openRegisteredContextMenu<TContext>(
  event: ContextMenuTriggerEvent,
  id: string,
  context: TContext,
) {
  const buildItems = registry.get(id) as ContextMenuFactory<TContext> | undefined;
  if (!buildItems) return false;
  return openContextMenu(event, buildItems(context), { menuId: id });
}

export function openDefinedContextMenu<TContext>(
  event: ContextMenuTriggerEvent,
  definition: ContextMenuDefinition<TContext>,
  context: TContext,
) {
  return openContextMenu(event, definition.buildItems(context), {
    menuId: definition.id,
  });
}

export function openContextMenu(
  event: ContextMenuTriggerEvent,
  items: ContextMenuItem[],
  options: { menuId?: string } = {},
) {
  event.preventDefault();
  event.stopPropagation();

  const normalized = normalizeItems(items);
  if (!normalized.some((item) => item.type !== "separator")) {
    closeContextMenu();
    return false;
  }

  window.dispatchEvent(
    new CustomEvent<ContextMenuOpenDetail>(OPEN_EVENT, {
      detail: {
        key: ++nextMenuKey,
        x: event.clientX,
        y: event.clientY,
        menuId: options.menuId,
        items: normalized,
      },
    }),
  );
  return true;
}

export function closeContextMenu() {
  window.dispatchEvent(new CustomEvent(CLOSE_EVENT));
}

export function ContextMenuHost() {
  const [menu, setMenu] = useState<OpenMenuState | null>(null);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const menuRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    const onOpen = (event: Event) => {
      const detail = (event as CustomEvent<ContextMenuOpenDetail>).detail;
      setPosition({ left: detail.x, top: detail.y });
      setMenu(detail);
    };
    const onClose = () => setMenu(null);

    window.addEventListener(OPEN_EVENT, onOpen);
    window.addEventListener(CLOSE_EVENT, onClose);
    return () => {
      window.removeEventListener(OPEN_EVENT, onOpen);
      window.removeEventListener(CLOSE_EVENT, onClose);
    };
  }, []);

  useEffect(() => {
    if (!menu) return;

    const close = () => setMenu(null);
    const onPointerDown = (event: PointerEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      close();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [menu]);

  useLayoutEffect(() => {
    if (!menu || !menuRef.current) return;

    const rect = menuRef.current.getBoundingClientRect();
    const maxLeft = window.innerWidth - rect.width - VIEWPORT_GUTTER;
    const maxTop = window.innerHeight - rect.height - VIEWPORT_GUTTER;
    const left = Math.max(VIEWPORT_GUTTER, Math.min(menu.x, maxLeft));
    const top = Math.max(VIEWPORT_GUTTER, Math.min(menu.y, maxTop));

    setPosition((current) =>
      current.left === left && current.top === top ? current : { left, top },
    );
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const frame = window.requestAnimationFrame(() => {
      const first = getEnabledItems(itemRefs.current)[0];
      (first ?? menuRef.current)?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [menu]);

  if (!menu) return null;

  const runItem = (item: ContextMenuActionItem) => {
    if (item.disabled) return;
    setMenu(null);
    try {
      const result = item.onSelect?.();
      if (result instanceof Promise) {
        void result.catch((error) => console.error(error));
      }
    } catch (error) {
      console.error(error);
    }
  };

  const focusItem = (offset: number) => {
    const buttons = getEnabledItems(itemRefs.current);
    if (!buttons.length) return;

    const currentIndex = buttons.findIndex((button) => button === document.activeElement);
    const nextIndex =
      currentIndex < 0
        ? offset > 0
          ? 0
          : buttons.length - 1
        : (currentIndex + offset + buttons.length) % buttons.length;

    buttons[nextIndex]?.focus();
  };

  const focusEdge = (edge: "first" | "last") => {
    const buttons = getEnabledItems(itemRefs.current);
    const target = edge === "first" ? buttons[0] : buttons[buttons.length - 1];
    target?.focus();
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setMenu(null);
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusItem(1);
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      focusItem(-1);
    }
    if (event.key === "Home") {
      event.preventDefault();
      focusEdge("first");
    }
    if (event.key === "End") {
      event.preventDefault();
      focusEdge("last");
    }
  };

  let actionIndex = -1;

  return (
    <div
      ref={menuRef}
      className="context-menu"
      style={{ left: position.left, top: position.top }}
      role="menu"
      aria-label={menu.menuId}
      tabIndex={-1}
      onKeyDown={onKeyDown}
    >
      {menu.items.map((item, index) => {
        if (item.type === "separator") {
          return (
            <div
              key={item.id ?? `separator-${index}`}
              className="context-menu-separator"
              role="separator"
            />
          );
        }

        actionIndex += 1;
        const refIndex = actionIndex;

        return (
          <button
            key={item.id}
            ref={(node) => {
              itemRefs.current[refIndex] = node;
            }}
            type="button"
            className={`context-menu-item${item.danger ? " danger" : ""}`}
            role="menuitem"
            disabled={item.disabled}
            onClick={() => runItem(item)}
          >
            <span className="context-menu-label">{item.label}</span>
            {item.hint && <span className="context-menu-hint">{item.hint}</span>}
          </button>
        );
      })}
    </div>
  );
}

function normalizeItems(items: ContextMenuItem[]) {
  const normalized: ContextMenuItem[] = [];
  let lastWasSeparator = true;

  for (const item of items) {
    if (item.type === "separator") {
      if (!lastWasSeparator && normalized.length > 0) {
        normalized.push(item);
        lastWasSeparator = true;
      }
      continue;
    }

    normalized.push(item);
    lastWasSeparator = false;
  }

  while (normalized[normalized.length - 1]?.type === "separator") {
    normalized.pop();
  }

  return normalized;
}

function getEnabledItems(items: Array<HTMLButtonElement | null>) {
  return items.filter(
    (item): item is HTMLButtonElement =>
      !!item && document.body.contains(item) && !item.disabled,
  );
}
