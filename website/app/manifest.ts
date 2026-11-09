import type { MetadataRoute } from "next";

export const dynamic = "force-static";

export default function manifest(): MetadataRoute.Manifest {
  return {
    name: "Aleph",
    short_name: "Aleph",
    description: "A language for agents to write backends that scale safely.",
    start_url: "/",
    display: "browser",
    background_color: "#0e1015",
    theme_color: "#5b4ce6",
    icons: [
      { src: "/icon.svg", sizes: "any", type: "image/svg+xml" },
      { src: "/apple-icon.png", sizes: "180x180", type: "image/png" },
    ],
  };
}
