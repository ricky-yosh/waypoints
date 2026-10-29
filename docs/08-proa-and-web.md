# Proa and the web side

## What Proa is

Proa (proa.so) is a new Rust web framework: batteries-included, server-rendered, Axum-native, with streaming SSR and Suspense, typed Markdown, and "RSJS" islands (interactivity written in Rust, compiled to a tiny JS runtime). Its pitch is multi-target rendering: one typed component renders to HTML and Markdown. Proa UI is a Shadcn-style set of Rust components (about 58); some are free (Accordion, Table, Button, Card, Field) and some are Pro/paid (Calendar, Chart, Code Block, Chat). Databases connect through a DataLoader (SQLx, Diesel, Toasty). Deploy guides cover Railway, Coolify, and Docker. Benchmarks on its site date from September 2026, so treat it as an early project.

## Proa is not a desktop UI tool

It does not replace GPUI. It is only for web pages.

## Where it could fit here (optional)

- A public gallery of shareable plan or graph templates.
- Run report pages for each plan execution. Because a component renders to HTML and Markdown, a report is both a page and a paste-able document.
- A small docs or landing site for the tool.

## Caveats

- It needs a running server (it is not a static site generator). Hosting options are Railway, Coolify on a VPS, or Docker.
- Offline use (for example a PWA with a service worker) is not documented in what was checked; verify before relying on it.
- Some useful components are paid.
- Not needed for the core app. The mobile and web question for this project is still open.

## Original question that started this

The author was building a TTRPG desktop app in Rust and GPUI, had heard Proa was good, and asked about a website or mobile companion. The table has no wifi, which is why the app is desktop and offline-first. The conversation then moved to a different project (this one).
