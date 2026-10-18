// The website: the home page (src/pages/index.astro) and the docs (Starlight, under /docs/).
// Built by `npm run build` here; Cloudflare runs it through wrangler.jsonc's build command.
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://noexplorer.vnappmob.com",
  // Pages load when a link is hovered, so moving around the docs feels instant.
  prefetch: { prefetchAll: true, defaultStrategy: "hover" },
  integrations: [
    starlight({
      title: "NoExplorer",
      description: "Docs for NoExplorer: drag files between your computer and your cloud storage.",
      logo: { src: "./src/assets/icon.svg", alt: "NoExplorer" },
      favicon: "/favicon.png",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/vnappmob/noexplorer" }],
      editLink: { baseUrl: "https://github.com/vnappmob/noexplorer/edit/main/site/" },
      customCss: ["./src/styles/docs.css"],
      // The site has its own 404 page (public/404.html).
      disable404Route: true,
      sidebar: [{ label: "Docs", items: [{ autogenerate: { directory: "docs" } }] }],
      head: [{ tag: "meta", attrs: { property: "og:image", content: "https://noexplorer.vnappmob.com/icon.png" } }],
    }),
  ],
});
