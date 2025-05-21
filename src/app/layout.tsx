// FILE: src/app/layout.tsx
// IMPORTANT NOTE: Rewrite the entire file.
"use client"; // Required for client-side hooks and context

import { Geist_Sans } from "geist/font/sans";
import { Geist_Mono } from "geist/font/mono";
import "@/styles/globals.css";
import { Toaster } from "@/components/ui/toaster"; // Assuming Shadcn UI Toaster

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className={`${Geist_Sans.variable} ${Geist_Mono.variable}`}>
      <body>
        <main className="min-h-screen"> {/* Ensure main content area can grow */}
          {children}
        </main>
        <Toaster /> {/* Global Toaster for notifications */}
      </body>
    </html>
  );
}