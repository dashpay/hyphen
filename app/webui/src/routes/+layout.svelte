<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { localeStore, tr } from "@repo/hyphen-i18n";
  import type { Snippet, Component } from "svelte";
  import { onMount } from "svelte";
  import "../app.scss";

  import {
    VwAppShell,
    VwDivider,
    VwHStack,
    VwHeader,
    VwHeaderDropdown,
    VwSidebar,
    VwSpacer,
    VwThrobber,
    VwText,
    VwToggle,
    VwVStack,
  } from "@repo/hyphen-views";
  import { explorer, loadWebUiConfig, webUiConfigError, webUiConfigLoading, getWebUiConfig } from "$lib";
  import {
    HyUtilGetLocaleOptions,
    HyUtilGetSelectedDenominationKey,
    HyUtilGetSelectedDenominationLabel,
    HyUtilGetSelectedLocaleLabel,
    HyUtilHydrateSelectedDenominationKey,
    HyUtilHydrateSelectedLocale,
    HyUtilRefreshLocaleDictionaries,
    HyUtilSetSelectedDenominationKey,
    HyUtilSetSelectedLocaleCode,
  } from "$lib/HyUtil";
  import {
    HyCenteredState,
    HyDropdownOption,
    HyMobileDock,
    HyNavItem,
    HySearchBox,
    HyShellBackground,
    HyShellError,
    HyShellLayout,
    HySidebarToggle,
  } from "$lib/views";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import IconHome from "virtual:icons/mdi/home-outline";
  import IconBlock from "virtual:icons/mdi/cube-outline";
  import IconTransactions from "virtual:icons/mdi/swap-horizontal";
  import IconBalances from "virtual:icons/mdi/wallet-outline";
  import IconValidators from "virtual:icons/solar/server-path-outline";
  import IconNetwork from "virtual:icons/mdi/lan-connect";
  import IconProposals from "virtual:icons/mdi/vote-outline";
  import IconSidebarHide from "virtual:icons/tabler/layout-sidebar-left-expand-filled";
  import IconSidebarShow from "virtual:icons/tabler/layout-sidebar-right-expand-filled";

  const THEME_STORAGE_KEY = "hyphen-webui-theme-light";
  const SIDEBAR_STORAGE_KEY = "hyphen-webui-sidebar-collapsed";

  interface NavItem {
    readonly id: string;
    readonly label: string;
    readonly href: string;
    readonly icon: Component;
    readonly featureKey?: "governance" | "validators";
    readonly indexKey?: "address" | "validator";
  }

  const { children }: { children: Snippet } = $props();
  const activeLocale = $derived($localeStore);

  const navItems = $derived.by((): readonly NavItem[] => {
    void activeLocale;

    return [
      { id: "overview", label: tr("Overview"), href: "/", icon: IconHome },
      { id: "blocks", label: tr("Blocks"), href: "/blocks", icon: IconBlock },
      { id: "transactions", label: tr("Transactions"), href: "/transactions", icon: IconTransactions },
      { id: "balances", label: tr("Balances"), href: "/balances", icon: IconBalances, indexKey: "address" },
      {
        id: "validators",
        label: tr("Validators"),
        href: "/validators",
        icon: IconValidators,
        featureKey: "validators",
        indexKey: "validator",
      },
      { id: "network", label: tr("Network"), href: "/network", icon: IconNetwork },
      {
        id: "proposals",
        label: tr("Proposals"),
        href: "/proposals",
        icon: IconProposals,
        featureKey: "governance",
      },
    ];
  });

  const navigationReady = $derived.by(
    (): boolean => !webUiConfigLoading() && (explorer.loadState === "ready" || explorer.loadState === "error")
  );

  const visibleNavItems = $derived.by((): readonly NavItem[] => {
    if (!navigationReady) {
      return [];
    }

    return navItems.filter((item): boolean => isAvailable(item));
  });

  const mobileDockItems = $derived.by(
    (): readonly { active: boolean; href: string; icon: Component; label: string }[] => {
      return visibleNavItems.map((item) => ({
        active: isActive(item),
        href: item.href,
        icon: item.icon,
        label: item.label,
      }));
    }
  );

  const denominationOptions = $derived.by((): { value: string; label: string }[] => {
    if (webUiConfigLoading()) {
      return [];
    }

    return getWebUiConfig().currency.denominations.map((denomination) => ({
      value: denomination.key,
      label: denomination.label,
    }));
  });

  const localeOptions = $derived.by(() => {
    void activeLocale;
    return HyUtilGetLocaleOptions();
  });
  const selectedLocaleLabel = $derived.by((): string => {
    void activeLocale;
    return HyUtilGetSelectedLocaleLabel();
  });

  const selectedDenominationKey = $derived.by((): string | undefined => {
    if (webUiConfigLoading()) {
      return undefined;
    }

    return HyUtilGetSelectedDenominationKey(getWebUiConfig());
  });

  let lightMode = $state(false);
  let dockMode = $state(false);
  let searchExpanded = $state(false);
  let sidebarCollapsed = $state(false);

  const searchTargets = $derived.by((): readonly string[] => {
    void activeLocale;

    const targets = [tr("block"), tr("transaction"), tr("address")];

    if (isFeatureEnabled("validators")) {
      targets.push(tr("validator"));
    }

    if (isFeatureEnabled("governance")) {
      targets.push(tr("proposal"));
    }

    return targets;
  });

  const searchPlaceholder = $derived.by((): string => {
    void activeLocale;

    if (dockMode) {
      return tr("Search");
    }

    return tr("Search {}", searchTargets.join(", "));
  });
  const searchAriaLabel = $derived(tr("Search {}", searchTargets.join(", ")));

  $effect(() => {
    globalThis.document.documentElement.classList.toggle("theme-light", lightMode);
  });

  function handleThemeToggle(checked: boolean): void {
    lightMode = checked;

    if (typeof globalThis.localStorage !== "undefined") {
      globalThis.localStorage.setItem(THEME_STORAGE_KEY, checked ? "1" : "0");
    }
  }

  function setSidebarCollapsed(nextValue: boolean): void {
    sidebarCollapsed = nextValue;

    if (typeof globalThis.localStorage !== "undefined") {
      globalThis.localStorage.setItem(SIDEBAR_STORAGE_KEY, nextValue ? "1" : "0");
    }
  }

  function isActive(item: NavItem): boolean {
    if (item.href === "/") {
      return page.url.pathname === "/";
    }

    return page.url.pathname.startsWith(item.href);
  }

  function hasAddressIndex(): boolean {
    const status = explorer.status;
    if (status == null) {
      return false;
    }

    return status.indices["script"] != null && status.indices["spent"] != null;
  }

  function hasValidatorIndex(): boolean {
    const status = explorer.status;
    if (status == null) {
      return false;
    }

    return status.indices["validator"] != null;
  }

  function isFeatureEnabled(featureKey: "governance" | "validators"): boolean {
    if (webUiConfigLoading()) {
      return false;
    }

    return getWebUiConfig().features[featureKey].enabled;
  }

  function isAvailable(item: NavItem): boolean {
    if (item.featureKey != null && !isFeatureEnabled(item.featureKey)) {
      return false;
    }

    if (item.indexKey == null) {
      return true;
    }

    if (item.indexKey === "address") {
      return hasAddressIndex();
    }

    return hasValidatorIndex();
  }

  function formatUpdatedAt(): string {
    const status = explorer.status;

    if (status == null) {
      return tr("Loading...");
    }

    const latestUpdatedAt = Object.values(status.indices).reduce((latestTimestamp: number, indexStatus): number => {
      const timestamp = Date.parse(indexStatus.updatedAt);
      return Number.isNaN(timestamp) ? latestTimestamp : Math.max(latestTimestamp, timestamp);
    }, 0);

    if (latestUpdatedAt === 0) {
      return tr("Last updated unavailable.");
    }

    return tr("Last updated {}", new Date(latestUpdatedAt).toISOString().replace("T", " ").replace(".000Z", " UTC"));
  }

  onMount((): (() => void) => {
    let mounted = true;
    const stopObservingDockMode = observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
      if (!nextValue) {
        searchExpanded = false;
      }
    });

    const storedTheme = globalThis.localStorage.getItem(THEME_STORAGE_KEY);
    const storedSidebar = globalThis.localStorage.getItem(SIDEBAR_STORAGE_KEY);

    lightMode = storedTheme === "1";
    sidebarCollapsed = storedSidebar === "1";

    HyUtilHydrateSelectedLocale();

    void loadWebUiConfig()
      .then((): void => {
        if (!mounted) {
          return;
        }

        HyUtilRefreshLocaleDictionaries();
        HyUtilHydrateSelectedDenominationKey(getWebUiConfig());
        explorer.init();
      })
      .catch((): void => {
        if (!mounted) {
          return;
        }

        explorer.reset();
      });

    return (): void => {
      mounted = false;
      stopObservingDockMode();
      explorer.reset();
    };
  });

  $effect((): void => {
    if (webUiConfigLoading() || explorer.loadState !== "ready") {
      return;
    }

    if (page.url.pathname.startsWith("/validators") && !hasValidatorIndex()) {
      void goto("/", { replaceState: true });
      return;
    }

    if (page.url.pathname.startsWith("/validators") && !isFeatureEnabled("validators")) {
      void goto("/", { replaceState: true });
      return;
    }

    if (page.url.pathname.startsWith("/validator") && !isFeatureEnabled("validators")) {
      void goto("/", { replaceState: true });
      return;
    }

    if (page.url.pathname.startsWith("/proposals") && !isFeatureEnabled("governance")) {
      void goto("/", { replaceState: true });
      return;
    }

    if (page.url.pathname.startsWith("/balances") && !hasAddressIndex()) {
      void goto("/", { replaceState: true });
    }
  });
</script>

<svelte:head>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="anonymous" />
  <link
    href="https://fonts.googleapis.com/css2?family=Huninn&family=JetBrains+Mono:wght@400;500;700&family=Manrope:wght@300;500;600;800&family=Space+Grotesk:wght@300;400;500;600;700&display=swap"
    rel="stylesheet"
  />
</svelte:head>

<VwAppShell>
  {#snippet sidebar()}
    <VwSidebar collapsed={sidebarCollapsed}>
      <VwHStack align="center" gap={12} justify={sidebarCollapsed ? "center" : "between"}>
        {#if !sidebarCollapsed}
          <VwText as="h1" size="lg" weight="extrabold">Hyphen</VwText>
        {/if}

        <HySidebarToggle
          icon={sidebarCollapsed ? IconSidebarShow : IconSidebarHide}
          label={sidebarCollapsed ? tr("Expand sidebar") : tr("Collapse sidebar")}
          onclick={(): void => {
            setSidebarCollapsed(!sidebarCollapsed);
          }}
        />
      </VwHStack>

      <VwSpacer size={8} />

      <VwVStack gap={4}>
        {#if navigationReady}
          {#each visibleNavItems as item (item.id)}
            <HyNavItem
              active={isActive(item)}
              collapsed={sidebarCollapsed}
              href={item.href}
              icon={item.icon}
              label={item.label}
            />
          {/each}
        {/if}
      </VwVStack>

      <VwSpacer />

      {#if !sidebarCollapsed}
        <VwDivider opacity={0.15} />

        {#if webUiConfigError() != null}
          <VwText as="p" color="muted" size="xs">{webUiConfigError()}</VwText>
        {:else}
          <VwText as="p" color="muted" size="xs">
            {formatUpdatedAt()}<br />
            {#if explorer.status != null}
              {tr("Powered by Hyphen v{}.", explorer.status.version)}
            {/if}
          </VwText>
        {/if}
      {/if}
    </VwSidebar>
  {/snippet}

  <HyShellLayout>
    {#snippet header()}
      <VwHeader>
        {#snippet left()}
          <div class="hy-shell-header-search" class:dock-mode={dockMode} class:expanded={dockMode && searchExpanded}>
            <HySearchBox
              ariaLabel={searchAriaLabel}
              disabled={webUiConfigLoading() || explorer.loadState === "loading"}
              governanceEnabled={isFeatureEnabled("governance")}
              hasAddressIndex={hasAddressIndex()}
              hasValidatorIndex={hasValidatorIndex()}
              onfocuschange={(nextValue: boolean): void => {
                searchExpanded = nextValue;
              }}
              placeholder={searchPlaceholder}
              validatorsEnabled={isFeatureEnabled("validators")}
            />
          </div>
        {/snippet}

        {#snippet right()}
          <div
            class="hy-shell-header-controls"
            class:dock-mode={dockMode}
            class:search-expanded={dockMode && searchExpanded}
          >
            <div class="hy-shell-header-controls-stack">
              <div class="hy-shell-header-prefs" class:hidden={dockMode && searchExpanded}>
                <VwHeaderDropdown label={selectedLocaleLabel}>
                  {#each localeOptions as option (option.code)}
                    <HyDropdownOption
                      onclick={(): void => {
                        HyUtilSetSelectedLocaleCode(option.code);
                      }}
                    >
                      <VwText size="sm">{option.label}</VwText>
                    </HyDropdownOption>
                  {/each}
                </VwHeaderDropdown>

                {#if selectedDenominationKey != null && denominationOptions.length > 0}
                  <VwHeaderDropdown label={HyUtilGetSelectedDenominationLabel(getWebUiConfig())}>
                    {#each denominationOptions as option (option.value)}
                      <HyDropdownOption
                        onclick={(): void => {
                          HyUtilSetSelectedDenominationKey(option.value);
                        }}
                      >
                        <VwText size="sm">{option.label}</VwText>
                      </HyDropdownOption>
                    {/each}
                  </VwHeaderDropdown>
                {/if}
              </div>

              <div class="hy-shell-header-theme">
                <VwToggle checked={lightMode} onchange={handleThemeToggle} />
              </div>
            </div>
          </div>
        {/snippet}
      </VwHeader>
    {/snippet}

    {#snippet body()}
      {#if page.url.pathname === "/whoops" || page.url.pathname === "/not-found"}
        {@render children()}
      {:else if webUiConfigLoading() || explorer.loadState === "idle" || explorer.loadState === "loading"}
        <HyCenteredState><VwThrobber /></HyCenteredState>
      {:else if webUiConfigError() != null}
        <HyShellError message={webUiConfigError() ?? tr("Unknown error")} title={tr("Failed to load configuration")} />
      {:else if explorer.loadState === "error" && explorer.error != null}
        <HyShellError message={explorer.error} title={tr("Failed to connect to API")} />
      {:else}
        {@render children()}
      {/if}
    {/snippet}
  </HyShellLayout>
  <HyMobileDock items={mobileDockItems} />
</VwAppShell>

<HyShellBackground />

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .hy-shell-header-search,
  .hy-shell-header-controls {
    min-width: 0;
  }

  .hy-shell-header-controls-stack {
    align-items: center;
    display: flex;
    gap: 12px;
    min-width: 0;
  }

  .hy-shell-header-prefs {
    align-items: center;
    display: flex;
    flex-shrink: 1;
    gap: 12px;
    min-width: 0;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .hy-shell-header-prefs.hidden {
      display: none;
    }

    .hy-shell-header-controls.search-expanded .hy-shell-header-controls-stack {
      gap: 8px;
    }

    .hy-shell-header-prefs > * {
      min-width: 0;
    }

    .hy-shell-header-controls.search-expanded :global(.dropdown-trigger) {
      font-size: var(--text-sm);
      gap: 4px;
      padding: 8px 12px;
    }

    .hy-shell-header-prefs :global(.dropdown-menu) {
      left: auto;
      right: 0;
    }

    :global(.header .inner:has(.hy-shell-header-search.expanded)) {
      gap: 8px;
    }

    :global(.header .left:has(.hy-shell-header-search.expanded)) {
      flex: 1 1 auto;
    }

    :global(.header .right:has(.hy-shell-header-controls.search-expanded)) {
      flex-shrink: 1;
    }
  }
</style>
