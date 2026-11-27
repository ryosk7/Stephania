# T3Box design

How T3Box's interfaces look and how their code is organised. It covers
T3Box Connect (the phone app, `apps/mobile`, iOS first) in detail, and the
direction for the Studio (`apps/studio`, web). Every rule here comes from
the research in §2 or from a cited guideline, not from taste. When a rule
and a screen disagree, change the screen or change this file with a reason.

## 1. What the interfaces are for

- **Connect** sets a T3Box up (QR, Bluetooth, Wi-Fi), operates a show
  (start/stop, scenes, audio) and turns the phone into one or two cameras.
  It is used standing next to a stage, often in a dark room, with one hand,
  while something is live. A mistake is public.
- **Studio** is the desk: scenes, sources, overlays, Preview/Program on a
  laptop.

Two consequences drive everything below: the current state of the
broadcast must be visible at a glance on every screen, and the one action
that matters on a screen must be the easiest to hit and the hardest to hit
by accident.

## 2. Research

Screens studied on Mobbin (iOS unless noted). What each contributes:

| Reference | Pattern taken |
|---|---|
| [Shopee live setup](https://mobbin.com/screens/f3a480cf-548d-4968-815f-d9e16264aefb), [X](https://mobbin.com/screens/2946114c-edc3-4fd6-a75b-a86611cd2ed5), [Azar](https://mobbin.com/screens/a5f8b3f6-1c81-433e-a5b9-07c66e6c7fae), [Yubo](https://mobbin.com/screens/a62d8f0d-9c4d-4b51-9455-902f79fda862) | Camera fills the screen on black; close (×) top-left; **one full-width pill "Go live" at the bottom**; secondary controls as a row of round icon buttons above it |
| [Citizen live](https://mobbin.com/screens/30233aa8-b544-49df-95a2-88f39cda7a88), [Substack live](https://mobbin.com/screens/9b2b7b68-c0e4-48bc-ab52-9f0eddfc73f2), [Insight Timer](https://mobbin.com/screens/e0bb88d9-9fbf-41ea-80fc-dc295086cf4c) | While live: a small red **LIVE** badge plus elapsed time / count at the top, the stop control red and isolated |
| [GoPro Quik](https://mobbin.com/screens/8230cc25-2a11-449b-8a9e-4eaa019dc029) ([busy](https://mobbin.com/screens/fc82756d-9bb7-4abd-9ca8-c3fca9104d66), [recording](https://mobbin.com/screens/463452e6-646c-4cb0-b426-458b29d58b22), [preferences](https://mobbin.com/screens/bf5ddc1a-9713-4d37-81fa-7fde2c57f823)) | Device-control app on near-black (#1C1C1E-like) surfaces; **device status strip** (storage, Wi-Fi, battery) at the top; a state pill with a coloured dot; the current mode as a chip with a spec line ("1080 \| 24 \| W"); settings as dark grouped lists with uppercase section headers and right-aligned values |
| [Tesla](https://mobbin.com/screens/c0a37ffb-23c3-44d7-a3e8-fd50735582e0) ([home](https://mobbin.com/screens/e9185aa7-9613-424c-b995-08f17e9e2ae3), [climate](https://mobbin.com/screens/85d767fe-f07a-49da-8b4f-ca4e7e2a26e9)) | **Device name as the large title with a chevron** (device switcher); a one-line live status under it; a row of icon quick actions; a detail card with a dotted spec line; segmented control for mutually exclusive modes |
| [Sonos](https://mobbin.com/screens/fa406e3f-4757-4405-8e1a-c14dd796da4f) ([system](https://mobbin.com/screens/e65bd2b5-534a-428e-ae83-dbc030ecdbca)) | A **persistent bottom bar for what is playing now** with its key control, above every other screen of the app — our "On air" bar |
| [Amazon Alexa Wi-Fi setup](https://mobbin.com/flows/ae937a1f-5150-4d20-82be-f9aa49fbbfd9) | Setup as a sequence of **single-purpose status screens**: centred symbol, title, one sentence; form steps with a large title and a bottom primary button; success with a check and paired secondary/primary buttons |
| [Muse connect a device](https://mobbin.com/flows/b4f8f7a2-0b88-422e-b925-0389574d9103) | Discovery as an inset rounded card that says "Searching…" with a spinner, then "No devices found · Scan again" with a hint; round glass back/close buttons |
| [SmartThings onboarding](https://mobbin.com/flows/997a7e22-2a7a-4ad4-ba77-928b75e9a425) | Home empty state as a card with one call to action ("Start setting up … with QR code") |
| [ChatGPT pair](https://mobbin.com/screens/86025ce1-6692-49bb-bd25-d21c47948d4c), [Apple Watch pair](https://mobbin.com/screens/920b4483-a3d0-4b6a-87ba-787a75f59157), [Tonal](https://mobbin.com/screens/f5445110-b0ea-48c9-999b-b0ae40981ffc), [BeReal](https://mobbin.com/screens/d3bf4c32-c7b1-495e-a18e-7415ce22df8f) | QR pairing: dark viewfinder with **corner brackets**, an instruction below it, and a quiet secondary way out ("Pair manually instead") |
| [Skype scenes](https://mobbin.com/screens/95169ea7-176b-458e-9a2e-964abb3103d9), [Replika scenes](https://mobbin.com/screens/7de7ede7-e836-465d-a3df-75de33050200), [Calm](https://mobbin.com/screens/1f4cae6a-e51c-4946-987b-400f4a53d8b3) | Scene choice as a grid of 16:9 tiles with the name on the tile; the selected tile is outlined, not recoloured |
| [Clubhouse live setup](https://mobbin.com/screens/80914864-4988-405b-b675-fde1d2dcf91a), [eBay edit scene](https://mobbin.com/screens/ea3dd161-9ade-469b-ac87-bcc2a87ac3cd) | Options for an action in a **bottom sheet** with a grabber, chips, and the action as the sheet's last element |
| Web: [Vimeo live studio](https://mobbin.com/screens/6caeef01-524d-4452-b645-36f30e9a6967), [Riverside](https://mobbin.com/screens/624780e9-f076-4a72-8686-4cd3f6a352f8), [Circle backstage](https://mobbin.com/screens/374283ae-ffaf-4e26-baa7-1f9244f2b8c8), [Substack live](https://mobbin.com/screens/cead5524-3612-4935-ac7c-9c9e5529cb28), [Descript](https://mobbin.com/screens/f4107ec6-5455-4d41-bbbc-e2e80c221f44) | Studio layout: left panel (sources/"Backstage ⇄ On stage"), centre canvas with a **scene strip** of thumbnails under it, right inspector of collapsible sections, labelled icon rail; "Go live" as the single dark/accent button top-right; LIVE + timer centred at the top; "Backstage" label on what is not on air |

Guidelines: Apple HIG — hit targets at least 44×44 pt
([design tips](https://developer.apple.com/design/tips)); Expo Router —
`src/app` holds routes only, other code lives outside it
([core concepts](https://docs.expo.dev/router/basics/core-concepts/)).

## 3. Principles

1. **State first.** Every screen that belongs to a device shows that
   device's broadcast state (Off / Starting / Live / Error) in the same
   place. "Live" is shown only when the device reports it; a request that
   was merely accepted is "Starting".
2. **One primary action, at the bottom, full width.** Destructive actions
   (Stop stream, Stop sending) are red and never sit next to a primary
   action of the same size.
3. **Dark broadcast surface.** All of Connect is dark: it is used in dark
   rooms, camera previews read better on black, and every reference above
   that controls hardware or video is dark.
4. **Colour means state, not decoration.** Red = on air or stop, amber =
   degraded / needs attention, green = healthy / done, blue = in progress.
   Neutral greys for everything else. No brand accent competes with red.
5. **Degrade visibly.** When quality is reduced (thermal plan, front
   camera off, reconnecting) the screen says so in words next to the thing
   affected.
6. **Native first.** System font (SF Pro), SF Symbols, system haptics,
   standard navigation; nothing that fights iOS conventions.

## 4. Tokens (`apps/mobile/src/shared/ui/tokens.ts`)

Colours are iOS dark-appearance system colours so the app sits naturally on
iOS; names are semantic so Android can map them to its own palette.

| Token | Value | Use |
|---|---|---|
| `bg` | `#000000` | screen background, camera letterbox |
| `surface` | `#1C1C1E` | cards, grouped list rows, bars |
| `surfaceRaised` | `#2C2C2E` | controls on a surface (segmented track, icon buttons) |
| `separator` | `#38383A` | hairlines between rows |
| `text` | `#FFFFFF` | primary text |
| `textSecondary` | `#EBEBF5` @ 60 % | secondary text |
| `textTertiary` | `#EBEBF5` @ 30 % | placeholders, disabled |
| `live` | `#FF453A` | LIVE badge, stop, on-air dot |
| `warning` | `#FFD60A` | degraded, take-over prompt |
| `success` | `#30D158` | connected, done |
| `info` | `#0A84FF` | in progress, links |
| `onPrimary` | `#000000` | text on the white primary button |

Type (points, iOS text styles): `largeTitle` 34/41 bold · `title` 22/28
bold · `headline` 17/22 semibold · `body` 17/22 · `callout` 16/21 ·
`footnote` 13/18 · `caption` 12/16 · `mono` 13/18 Menlo (fingerprints,
URLs). Text scales with Dynamic Type (`allowFontScaling` stays on).

Spacing on a 4-pt grid: `xs 4 · s 8 · m 12 · l 16 · xl 24 · xxl 32`.
Screen padding 16. Radius: `card 16`, `tile 12`, `control 10`, `pill 999`.
Primary button height 54. Minimum hit target 44.

Motion: state changes cross-fade 150 ms; nothing animates while live except
the on-air dot. Haptics: `success` notification when going live,
`warning` on take-over, light impact on segment change.

## 5. Components (`apps/mobile/src/shared/ui`)

Built by composition (§7): small parts that combine, explicit variants
instead of boolean modes.

| Component | API | Notes |
|---|---|---|
| `Screen` | `<Screen scroll? footer={…}>` | dark background, safe areas; `footer` holds the bottom actions |
| `ImmersiveScreen` | `<ImmersiveScreen footer={…}>` | variant without a navigation header (camera) |
| Text | `<Title>`, `<LargeTitle>`, `<Headline>`, `<Body>`, `<Footnote>`, `<Caption>`, `<Mono>` | one component per text style, `tone="secondary"\|"live"\|…` |
| Buttons | `<PrimaryButton>`, `<SecondaryButton>`, `<DestructiveButton>`, `<TextButton>`, `<IconButton symbol>` | variants are separate components; all ≥ 44 pt |
| `List` | `<List.Section title footer>` `<List.Row title value? detail? onPress? accessory?>` | dark inset grouped list (GoPro preferences, Muse devices) |
| `Segmented` | `<Segmented value onChange><Segmented.Item value label disabled?/></Segmented>` | compound; shared state through context |
| `StatusPill` | `<StatusPill tone label pulse?>` | coloured dot + label (GoPro "Busy", Tesla status) |
| `LiveBadge` | `<LiveBadge elapsed?>` | red "LIVE" + time |
| `StatusView` | `<StatusView symbol title body tone>{actions}</StatusView>` | centred single-purpose state (Alexa setup steps, empty states) |
| `Card` | `<Card>`, `<Card.Header title action?>`, `<Card.Body>` | surface with radius 16 |
| `Notice` | `<Notice tone>` | inline sentence with a symbol: degradation, errors |
| `Field` | `<Field label secure? …TextInput props>` | filled input, label above |
| `Symbol` | `<Symbol name={{ios, android}} size tone>` | wraps `expo-symbols` with token colours |

## 6. Screens (Connect)

- **Home** — large title "T3Box"; one `Card` per enrolled device (name,
  status pill, "Control" and "Camera" actions); empty state = SmartThings
  pattern: card "Set up a T3Box · Scan the setup QR code" with one button;
  footer `TextButton` "Connect to a registered T3Box".
- **Setup** (stack, Alexa sequence; routes `setup/scan`, `setup/nearby`,
  `setup/wifi`): *Scan* — dark viewfinder with corner
  brackets, instruction under it, `TextButton` "Find it nearby instead";
  *Nearby* — Muse-style searching card, then a `List` of devices with
  signal strength; *Wi-Fi* — large title, `List` of networks with lock
  symbols, `Field` for the password, `PrimaryButton` "Join", then progress
  states (joining, issuing access); *Done* — `StatusView` with a check,
  `PrimaryButton` "Open controls".
- **Control** (`device/[id]`) — Tesla header: device name as large title
  with a rename button (the name lives on this phone only: host name,
  certificate and mDNS name stay, so nothing is set up again; rename is a
  bottom sheet, not `Alert.prompt`, which Android lacks), the host name
  below it, then the status pill. **Layouts** (Studio scenes, named for
  what they are to an operator: "what viewers see"): a 2-column grid of
  16:9 miniatures drawn from each scene's own layers (slides, cameras,
  overlays), name under the tile; the on-air tile is outlined red with an
  ON AIR tag, which is why there is no separate "on air" card. Tiles are
  always half a row, whatever the count (1 and 3 leave the last cell
  empty rather than stretching); more than fit scroll with the screen
  while the footer stays pinned; none = a card saying HDMI goes on air as
  it is and layouts are made in the Studio. Revision numbers are not shown
  (an editing detail, kept in the Studio). The fade switch sits on the
  section header. Audio as a `List` with switches; footer:
  `PrimaryButton` "Start stream" or `DestructiveButton` "Stop stream"
  (confirmed with a system alert, since stopping is public).
- **Camera** (`device/[id]/camera`, full-screen modal) — GoPro/live-app pattern: previews on black;
  top: close, `StatusPill` (Ready / Connecting / Live / Reconnecting (n) /
  Paused) and thermal `Notice`; previews labelled "Presenter · rear" /
  "Audience · front"; bottom panel: `Segmented` Rear / Front / Both
  (Both disabled with a reason when no dual profile), footer
  `PrimaryButton` "Go live" or `DestructiveButton` "Stop sending";
  when another phone holds the session the bottom panel turns into a
  sheet-like card: `Notice tone="warning"` with Cancel / Take over.
- **Open on PC** (`device/[id]/pc`) — `Mono` URL and fingerprint in a `Card`, steps as a
  numbered `List`, `PrimaryButton` "Share".

## 7. Code architecture (Connect)

Three rules, each from a researched source:

**Feature-based.** Code is grouped by feature, not by file type
(bulletproof-react
[project structure](https://github.com/alan2207/bulletproof-react/blob/master/docs/project-structure.md)).
`src/app` stays routes-only, as Expo Router requires; route files are thin
and only render a feature's screen.

```
src/
  app/                 routes only (Expo Router); each file renders one feature screen
  features/
    home/              enrolled devices, empty state
    setup/             scan, nearby (BLE), Wi-Fi, done  + ble/ transport
    control/           program, scenes, audio
    camera/            camera screen + session/plan/sdp (platform-neutral policy)
    pc-access/         open on a PC
  shared/
    ui/                tokens and the components of §5
    device/            DeviceClient, enrollment storage, error text, broadcast state, DeviceGate
```

Imports point one way: `app → features → shared`. A feature never imports
another feature; what two features need moves to `shared`. `shared` never
imports `features` or `app`. `src/architecture.test.ts` enforces this in
CI (the same boundaries bulletproof-react enforces with
`import/no-restricted-paths`).

**Composition.** Components expose parts, not flags: compound components
share state through context (`Segmented`, `List`, `Card`), variants are
separate components (`PrimaryButton` vs `DestructiveButton`), and children
are preferred over `renderX` props (Vercel
[composition patterns](https://github.com/vercel-labs/agent-skills): avoid
boolean-prop proliferation, use compound components, explicit variants).

**Colocation.** "Place code as close to where it's relevant as possible"
([Kent C. Dodds](https://kentcdodds.com/blog/colocation)): a feature's
components, hooks, styles, tests and fixtures live in its folder, tests
next to the file they test (`session.ts` + `session.test.ts`). Code moves
to `shared` only when a second feature needs it.

**Android.** Features talk to native code only through platform contracts
(`modules/t3box-camera/src/T3BoxCamera.types.ts`, the pinned fetch);
symbols name both platforms (`{ios, android}`); tokens are semantic. The
Android port implements the contracts and maps the tokens; screens and
features do not change.

## 8. Accessibility and copy

- Every control ≥ 44×44 pt; icon-only buttons have `accessibilityLabel`.
- State is never colour-only: pills and badges always carry a word.
- Text respects Dynamic Type; layouts wrap instead of truncating labels.
- Copy: English, sentence case, verbs on buttons ("Go live", "Join"),
  numbers with units, no exclamation marks; errors say what to do next.

## 9. Studio (web) direction

From the web references: three panes (left: sources and scenes with
"Backstage / On air" grouping; centre: Preview and Program canvases with
the scene strip below; right: inspector of collapsible sections), a
labelled icon rail, the broadcast state (LIVE + elapsed, or "Backstage")
centred in the top bar and "Go live" as the only accent button at its
right. Same semantic colours as Connect, in a dark theme. The Studio adopts
the same feature-based layout (`apps/studio/src/features/*`) when it is
restyled; until then this section is the target, not a description of the
current Studio.
