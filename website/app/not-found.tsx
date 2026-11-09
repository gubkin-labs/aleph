import Link from "next/link";

export const metadata = { title: "Page not found" };

export default function NotFound() {
  return (
    <main style={{ maxWidth: 640, margin: "0 auto", padding: "96px 16px", textAlign: "center" }}>
      <h1>Page not found</h1>
      <p>That page doesn&rsquo;t exist. Try the docs, or head back to the homepage.</p>
      <p>
        <Link href="/docs/">Docs</Link> · <Link href="/">Home</Link>
      </p>
    </main>
  );
}
