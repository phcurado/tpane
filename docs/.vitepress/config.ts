import { defineConfig } from "vitepress";

export default defineConfig({
  title: "tpane",
  description: "Lua-powered tmux config with plugins, themes, widgets, and pane helpers",
  base: "/tpane/",
  themeConfig: {
    logo: "/logo.png",
    search: { provider: "local" },
    sidebar: [
      {
        text: "Docs",
        items: [
          { text: "Quick start", link: "/quick-start" },
          { text: "Install", link: "/install" },
          { text: "Configuration", link: "/configuration" },
          { text: "Status bar", link: "/status-bar" },
          { text: "Plugins", link: "/plugins" },
          { text: "Reusable panes", link: "/reusable-panes" },
          { text: "Pane detection", link: "/pane-detection" },
          { text: "Lua API", link: "/lua-api" },
          { text: "Changelog", link: "/changelog" },
        ],
      },
    ],
  },
});
