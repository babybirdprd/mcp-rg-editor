"use client";
import { Geist_Sans } from "geist/font/sans"; // Corrected import if using Geist Vercel
import { Geist_Mono } from "geist/font/mono"; // Corrected import
import "@/styles/globals.css";
import { Toaster } from "@/components/ui/toaster"; // Import Toaster

// const geistSans = Geist({ // Original from template, might differ from Geist Vercel pkg
//   variable: "--font-geist-sans",
//   subsets: ["latin"],
// });

// const geistMono = Geist_Mono({
//   variable: "--font-geist-mono",
//   subsets: ["latin"],
// });

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className={`${Geist_Sans.variable} ${Geist_Mono.variable}`}> {/* Use classes directly if using geist/font */}
      <body>
        {children}
        <Toaster /> {/* Add Toaster here for global availability */}
      </body>
    </html>
  );
}