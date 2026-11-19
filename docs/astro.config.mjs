// The Open Desk Phone documentation site, built with Astro Starlight
// (https://starlight.astro.build/).  The pages are in src/content/docs/.
//
//   pnpm install
//   pnpm build      # writes dist/; fails on a type error (tsc) or a broken link
//   pnpm dev        # preview at http://localhost:4321
//
// The phone models the homepage turns (public/phones/) are built by
// tools/phone-models.

import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import rehypeRelativeMarkdownLinks from "astro-rehype-relative-markdown-links";
import remarkHeadingId from "remark-heading-id";
import starlightLinksValidator from "starlight-links-validator";

export default defineConfig({
  // Where the site is published (wrangler.jsonc), for the canonical
  // links and the sitemap.
  site: "https://opendeskphone.org",
  markdown: {
    // `## Heading {#id}` keeps an anchor that links rely on.
    remarkPlugins: [remarkHeadingId],
    // Pages link to each other by the Markdown file's relative path, e.g.,
    // `[Dev Guide](../dev-guide/index.md)`, which also works on GitHub.
    rehypePlugins: [[rehypeRelativeMarkdownLinks, { collectionBase: false, trailingSlash: "always" }]],
  },
  integrations: [
    starlight({
      title: "Open Desk Phone",
      description:
        "Open firmware for the Digium D-series desk phones, and Open DPMA, the module for Asterisk that provisions them.",
      // The boot screen's logo (assets/art/splash.svg), with the type as
      // outlines; "Open" is dark on the light theme.
      logo: { light: "./src/assets/logo-light.svg", dark: "./src/assets/logo-dark.svg", replacesTitle: true },
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/jackrosenthal/opendeskphone" }],
      customCss: ["./src/styles/custom.css"],
      plugins: [starlightLinksValidator()],
      // Every page is listed here, by the page's path under
      // src/content/docs/ without the extension.  A section's first page is
      // the section's index.  To add a page, add the page's path to the
      // section's list, in the order the pages are to be read.
      sidebar: [
        { label: "Home", link: "/" },
        {
          label: "Open DPMA",
          items: [
            "open-dpma",
            "open-dpma/installation",
            "open-dpma/configuration",
            "open-dpma/opendeskphone-options",
            "open-dpma/administration",
          ],
        },
        {
          label: "App SDK",
          items: [
            "app-sdk",
            "app-sdk/examples",
            {
              label: "Tutorial",
              items: [
                "app-sdk/tutorial",
                "app-sdk/tutorial/first-app",
                "app-sdk/tutorial/softkeys-and-state",
                "app-sdk/tutorial/persistent-storage",
                "app-sdk/tutorial/drawing",
                "app-sdk/tutorial/shared-data",
              ],
            },
            {
              label: "Reference",
              items: [
                "app-sdk/reference",
                "app-sdk/reference/manifest",
                "app-sdk/reference/app-events",
                "app-sdk/reference/types",
                "app-sdk/reference/helpers",
                {
                  label: "Interfaces",
                  items: [
                    "app-sdk/reference/runtime",
                    "app-sdk/reference/ui",
                    "app-sdk/reference/canvas",
                    "app-sdk/reference/timers",
                    "app-sdk/reference/storage",
                    "app-sdk/reference/config",
                    "app-sdk/reference/pbx",
                    "app-sdk/reference/http",
                    "app-sdk/reference/calls",
                    "app-sdk/reference/presence",
                    "app-sdk/reference/events",
                    "app-sdk/reference/audio",
                    "app-sdk/reference/l10n",
                    "app-sdk/reference/hotdesk",
                    "app-sdk/reference/diagnostics",
                    "app-sdk/reference/lamps",
                    "app-sdk/reference/input",
                    "app-sdk/reference/alarms",
                  ],
                },
              ],
            },
          ],
        },
        {
          label: "Dev Guide",
          items: [
            "dev-guide",
            "dev-guide/building",
            "dev-guide/simulator",
            "dev-guide/ssh-and-serial-console",
            "dev-guide/test-builds",
            "dev-guide/firmware-updates",
            "dev-guide/dphone-command",
            "dev-guide/diagnostics",
            "dev-guide/troubleshooting",
          ],
        },
      ],
    }),
  ],
});
