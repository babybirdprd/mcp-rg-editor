// FILE: src/app/page.tsx
// IMPORTANT NOTE: Rewrite the entire file.
"use client";
import { RoundedButton } from "@/components/RoundedButton"; // Assuming this is a custom button
import { invoke } from "@tauri-apps/api/core";
import Image from "next/image";
import Link from "next/link"; // For navigation
import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button"; // Assuming Shadcn UI Button

export default function Home() {
  const [greetMsg, setGreetMsg] = useState<string | null>(
    "Click the button to call a Rust function!",
  );
  const [isLoading, setIsLoading] = useState(false);

  const callGreet = useCallback(async () => {
    setIsLoading(true);
    try {
      const result = await invoke<string>("greet"); // Ensure 'greet' is registered in lib.rs
      setGreetMsg(result);
    } catch (error) {
      console.error("Error invoking greet:", error);
      setGreetMsg(`Error: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsLoading(false);
    }
  }, []);

  return (
    <div className="flex flex-col items-center justify-center min-h-screen p-4 font-[family-name:var(--font-geist-sans)]">
      <header className="mb-8 text-center">
        <Image
          className="dark:invert mx-auto mb-4"
          src="/next.svg" // Assuming this is in public folder
          alt="Next.js logo"
          width={180}
          height={38}
          priority
        />
        <h1 className="text-4xl font-bold">MCP-RG-Editor (Tauri Edition)</h1>
        <p className="text-lg text-muted-foreground">
          Enhanced Desktop Commander with Ripgrep, Filesystem, and Terminal tools.
        </p>
      </header>

      <nav className="mb-8">
        <Link href="/config" passHref>
          <Button variant="outline">Go to Configuration Page</Button>
        </Link>
      </nav>

      <section className="w-full max-w-md p-6 space-y-4 bg-card text-card-foreground rounded-lg shadow-md">
        <h2 className="text-2xl font-semibold">Test Backend Connection</h2>
        <p className="text-sm text-muted-foreground">
          {greetMsg}
        </p>
        <Button onClick={callGreet} disabled={isLoading} className="w-full">
          {isLoading ? "Calling..." : 'Call "greet" from Rust'}
        </Button>
      </section>

      <footer className="mt-12 text-center text-sm text-muted-foreground">
        <p>Powered by Tauri & Next.js</p>
        <div className="flex justify-center gap-4 mt-2">
          <a
            href="https://nextjs.org/docs"
            target="_blank"
            rel="noopener noreferrer"
            className="hover:underline"
          >
            Next.js Docs
          </a>
          <a
            href="https://tauri.app/v2/api/js/"
            target="_blank"
            rel="noopener noreferrer"
            className="hover:underline"
          >
            Tauri JS API
          </a>
        </div>
      </footer>
    </div>
  );
}