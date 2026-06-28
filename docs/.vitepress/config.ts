import { defineConfig } from "vitepress";

export default defineConfig({
  title: "tpane",
  description: "Configure tmux with Lua",
  base: "/tpane/",
  themeConfig: {
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
          { text: "Changelog", link: "https://github.com/phcurado/tpane/blob/main/CHANGELOG.md" },
        ],
      },
    ],
  },
});
