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
3. **Light and dark, following the system.** Connect has a light and a
   dark theme with the same structure and state colours, and follows the
   phone's setting by default; Settings → Appearance can fix it to Light
   or Dark (remembered on the phone). Dark suits dark rooms, light suits
   daylight setup. Camera viewfinders and previews stay dark in both: they
   are video.
4. **Colour means state, not decoration.** Red = on air or stop, amber =
   degraded / needs attention, green = healthy / done, blue = in progress.
   Neutral greys for everything else. No brand accent competes with red.
5. **Degrade visibly.** When quality is reduced (thermal plan, front
   camera off, reconnecting) the screen says so in words next to the thing
   affected.
6. **Platform-neutral.** iPhone and Android get the same components,
   drawn by T3Box: a bundled font (Inter, JetBrains Mono for addresses),
   one icon set (Lucide), our own top bar, switch, segmented control,
   sheets and confirmations — no SF Symbols, Material icons, system
   switches, native navigation headers or system alert dialogs. The
   research above supplies *patterns* (where things go, what states
   exist), not platform styling. What stays native is what only the system
   can do: keyboard, permission prompts, the share sheet, haptics, the
   status bar.
7. **Icons first, words where they matter.** Recurring controls are icons
   (toolbar actions, section headings, camera choice, audio on/off, state
   chips), so the screens scan without reading. Every icon-only control
   has a spoken label (`accessibilityLabel`) that is also shown on long
   press. Words stay on the one primary action and on anything that cannot
   be undone (icon plus one word: "Control", "Stop", "Join"), on names
   and addresses, and on errors. Explanations shrink to a short line or a
   picture (PC → QR) rather than a paragraph.

## 4. Tokens (`apps/mobile/src/global.css`)

T3Box's own palette — neutral greys and four state colours — identical on
both platforms, defined once as Tailwind CSS v4 theme variables with a
light and a dark value. Components use the semantic names only
(`bg-surface`, `text-secondary`, `border-live`), so a theme switch
changes every screen together.

| Token | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#F6F6F7` | `#0B0B0C` | screen background |
| `surface` | `#FFFFFF` | `#161618` | cards, lists, sheets, fields |
| `pressed` | black 4 % | white 4 % | pressed rows |
| `fill` | black 5 % | white 8 % | secondary and icon buttons, chips |
| `border` | black 9 % | white 8 % | 1 px card borders and row separators |
| `text` | `#18181B` | `#F4F4F5` | primary text, primary button, toggle on, selected segment |
| `secondary` | `#52525B` | `#A1A1AA` | secondary text, section icons |
| `tertiary` | `#A1A1AA` | `#71717A` | disabled, placeholders, chevrons |
| `on-primary` | `#FFFFFF` | `#0B0B0C` | text and icons on the primary button |
| `live` | `#E5372B` | `#F04438` | LIVE badge, stop, on-air ring |
| `warning` | `#B86E00` | `#F5B300` | degraded, unreachable, take-over |
| `success` | `#15803D` | `#22C55E` | done, connected |
| `info` | `#2563EB` | `#3B82F6` | in progress |
| `track-off`, `scrim`, `mini-*`, `video` | | | toggle off, sheet backdrop, layout miniatures, viewfinders (dark in both) |

The light state colours are darker than the dark ones so they keep their
contrast on white.

Type (Inter, bundled; weights are separate font files so Android does not
synthesise them): `display` 32/38 bold −0.6 · `title` 22/28 semibold ·
`headline` 16/22 semibold · `body` 16/22 · `label` 14/20 medium ·
`footnote` 13/18 · `overline` 11/14 semibold uppercase +0.8 (section
labels) · `mono` 13/18 JetBrains Mono (URLs, fingerprints). Text scales
with the system font size setting.

Spacing on Tailwind's 4-pt scale (`gap-2` = 8, `p-4` = 16).
Screen padding 16. Radius: `rounded-card` 16, `rounded-button` 14,
`rounded-control` 12, `rounded-tile` 12, `rounded-sheet` 24,
`rounded-full` (chips and badges only). Buttons are
rounded rectangles 52 high; minimum hit target 44.

Icons: Lucide, 2 px stroke on a 24 grid, 16–22 in controls; `stop` is
drawn solid. The names and their meaning are in `shared/ui/icon.tsx` and
the Icons story.

Motion: state changes cross-fade 150 ms; the toggle knob slides 150 ms;
nothing animates while live except the on-air dot. Haptics (both
platforms): `success` when going live, `warning` on take-over, `selection`
on toggles and segments.

## 5. Components (`apps/mobile/src/shared/ui`)

Built by composition (§7): small parts that combine, explicit variants
instead of boolean modes.

| Component | API | Notes |
|---|---|---|
| `Screen` | `<Screen header={<TopBar …/>} scroll? footer={…}>` | theme background, safe areas; `footer` holds the bottom actions |
| `TopBar` | `<TopBar onBack? backIcon? title? trailing?>` | app-drawn top bar: round back/close on the left, icon buttons on the right; native navigation headers are off |
| Text | `<Display>`, `<Title>`, `<Headline>`, `<Body>`, `<Label>`, `<Footnote>`, `<Overline>`, `<Mono>` | one component per text style, `tone="secondary"\|"live"\|…`, `className` for layout |
| Buttons | `<PrimaryButton>`, `<SecondaryButton>`, `<DestructiveButton>`, `<TextButton>` | variants are separate components; 52 high, radius 14; icon plus one word, `accessibilityLabel` for the full phrase; a forward arrow follows the word |
| `IconButton` | `<IconButton icon label shape="round"\|"inline"\|"square">` | icon-only control on `fill` with a 1 px border: round 44 (toolbars), inline 36 beside a title (44 hit area), square 52 beside a text button; `label` is spoken and shown on long press |
| `SectionHeading` | `<SectionHeading icon label aside?>` | section heading as an icon (the word is its accessible name); `aside` holds the section's control (the fade switch) |
| `List` | `<List.Section heading? footer>` `<List.Row title value? detail? leading? onPress? accessory?>`, `<ListIcon icon color?>` | rows (≥ 56) in a bordered card, full-width separators, leading icons on a neutral 36 rounded square, selection as a check |
| `Toggle` | `<Toggle value onChange label>` | drawn by T3Box (not the system switch): 48×28, track in the text colour when on |
| `Segmented` | `<Segmented value onChange locked?><Segmented.Item value label icon? unavailable?/></Segmented>` | compound; with `icon` an item shows only the icon and `label` is spoken |
| `StatusChip` | `<StatusChip tone icon label value? pulse?>` | state as an icon on the state colour plus a value (on-air time); the word is spoken and shown on long press |
| `LiveBadge` | `<LiveBadge elapsed?>` | red "LIVE" tag + tabular time |
| `StatusView` | `<StatusView icon title body tone>{actions}</StatusView>` | centred state: icon in a 64 ring, title, sentence, actions |
| `Card` | `<Card outlined?>`, `<Card.Header title>`, `<Card.Body>` | surface, radius 16, 1 px border |
| `Sheet` | `<Sheet visible title icon? onClose>{…}</Sheet>` | app-drawn bottom sheet: rename, stop confirmation (no system alerts) |
| `Notice` | `<Notice tone icon?>` | short line with an icon in a tinted, bordered box |
| `Field` | `<Field label icon? secure? …TextInput props>` | with `icon` the icon replaces the visible label (the label is spoken) |
| `Icon` | `<Icon name size tone? color?>` | Lucide icons by name; `tone` for state, `color` for any other token |

Every component has a story next to it (`buttons.stories.tsx`), shown in
both themes (§7).

## 6. Screens (Connect)

- **Home** — large title "T3Box" with three toolbar icons: Bluetooth
  (a registered T3Box nearby), plus (set up a T3Box), gear (Settings). One
  `Card` per enrolled device: name, `StatusChip` (broadcast icon + on-air
  time; Wi-Fi-off in amber when unreachable, with "Same Wi-Fi as this
  T3Box?"), a square camera button and `PrimaryButton` "Control". Empty
  state: card "Set up a T3Box" with the PC → QR picture and "Scan".
- **Settings** (`settings`) — Appearance: three preview tiles (system,
  light, dark, each a miniature screen in that theme) with the theme's
  icon under it; default is the system. About: Licences (every package that
  ships in the app with its licence text, `scripts/notices.mjs`) and the
  version.
- **Setup** (stack, Alexa sequence; routes `setup/scan`, `setup/nearby`,
  `setup/wifi`): *Scan* — dark viewfinder with corner
  brackets, the PC → QR picture and one line, `SecondaryButton`
  "Nearby"; *Nearby* — `List` of devices with signal bars, rescan and
  scan-code icons in the top bar; *Wi-Fi* — `List` of networks with lock
  and signal icons, rescan in the top bar, password `Field` with a lock
  icon, `PrimaryButton` "Join →", then progress states (joining, issuing
  access); *Done* — `StatusView` with a check, `PrimaryButton` "Control".
- **Control** (`device/[id]`) — Tesla header: device name as large title
  with a rename button (the name lives on this phone only: host name,
  certificate and mDNS name stay, so nothing is set up again; rename is a
  bottom sheet, not `Alert.prompt`, which Android lacks), the host name
  below it, then the `StatusChip`; camera and PC icons in the top bar.
  **Layouts** (section icon: grid) (Studio scenes, named for
  what they are to an operator: "what viewers see"): a 2-column grid of
  16:9 miniatures drawn from each scene's own layers (slides, cameras,
  overlays), name under the tile; the on-air tile is outlined red with an
  broadcast mark, which is why there is no separate "on air" card. Tiles are
  always half a row, whatever the count (1 and 3 leave the last cell
  empty rather than stretching); more than fit scroll with the screen
  while the footer stays pinned; none = a card "Make layouts in the
  Studio". Revision numbers are not shown (an editing detail, kept in the
  Studio). The fade switch (blend icon) sits on the section heading.
  Audio (section icon: speaker): HDMI and Mic rows whose icon shows on or
  muted, with switches; footer: `PrimaryButton` "Go live" or
  `DestructiveButton` "Stop" (confirmed in a `Sheet`, since stopping is
  public).
- **Camera** (`device/[id]/camera`, full-screen modal) — GoPro/live-app pattern: previews on black;
  top: close, `StatusChip` (Ready / Connecting / Live / Reconnecting n /
  Paused) and a short thermal `Notice` ("Warm · quality reduced");
  previews marked with the rear-camera or selfie icon; bottom panel:
  icon `Segmented` (rear, front, both; both disabled with a reason when
  there is no dual profile) over the resolution in mono, footer
  `PrimaryButton` "Go live" or `DestructiveButton` "Stop";
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
    settings/          appearance, licences
  shared/
    ui/                theme, appearance and the components of §5 (+ stories)
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

**Styling: Tailwind CSS v4 through Uniwind.** Components are styled with
`className` (Tailwind utilities on React Native components), compiled at
build time by [Uniwind](https://docs.uniwind.dev/) from `src/global.css`.
Uniwind was chosen over NativeWind because NativeWind's Tailwind v4
release (v5) is still a release candidate whose authors do not intend it
for production, while Uniwind 1.x is stable on Tailwind v4 and has
light / dark / system themes built in (`Uniwind.setTheme('system')`).
`shared/ui/theme.ts` applies and stores the user's choice; `useColor()`
gives the resolved value for props that need a colour (icons, spinners,
animated colours). Class strings are static so Tailwind sees them; tone
maps live in `tone.ts`.

**Storybook.** Every component in `shared/ui` (and feature components
worth seeing on their own) has a colocated story (`name.stories.tsx`),
checked by the architecture test. The same stories run in two
Storybooks: on the phone (`npm run storybook`, then the `/storybook`
route, compiled out of release builds) and on the web through
react-native-web (`npm run storybook:web`). CI builds the web one and
renders every story in both themes (`npm run check:stories`), uploading
the screenshots.

**Navigation.** Expo Router (file routes in `src/app`). The Studio, a
plain web app, uses TanStack Router (§9).

**Android.** Features talk to native code only through platform contracts
(`modules/t3box-camera/src/T3BoxCamera.types.ts`, the pinned fetch), and
every visible component is drawn by `shared/ui` with bundled fonts and
icons. The Android port implements the contracts; screens, features and
components do not change, and both platforms look the same.

## 8. Accessibility and copy

- Every control is drawn ≥ 44 and has a hit area ≥ 48 (`hitSlop` where drawn smaller), which covers Apple's 44 pt and Android's 48 dp guidance; icon-only buttons have `accessibilityLabel`; app-drawn controls set `accessibilityRole` (`switch`, `radio`, `button`) and their state.
- State is never colour-only: chips carry an icon of their own and a
  spoken word; icon-only controls have a spoken label, shown on long press.
- Text respects Dynamic Type; layouts wrap instead of truncating labels.
- Copy: English, sentence case, verbs on buttons ("Go live", "Join"),
  numbers with units, no exclamation marks; errors say what to do next.

## 9. Studio (web) direction

**Now in place:** the Studio uses the same semantic tokens as Connect in
Tailwind CSS v4 (`apps/studio/src/styles.css`), the same light / dark /
system appearance (a three-icon switch in the header, stored per browser,
system by default), Lucide icons beside page names and on icon-only
actions (with the word as the accessible name and tooltip), and TanStack
Router for its pages (hash history, so `#/studio`, `#/audio`,
`#/settings` keep working and the device serves one `index.html`).
Shell components have colocated stories in a Storybook (`@storybook/react-vite`)
that CI renders in both themes. No web fonts ship from the device
(docs/LICENSING.md): the Studio uses the system UI font. The program
canvas stays dark in both themes.

**Next:** From the web references: three panes (left: sources and scenes with
"Backstage / On air" grouping; centre: Preview and Program canvases with
the scene strip below; right: inspector of collapsible sections), a
labelled icon rail, the broadcast state (LIVE + elapsed, or "Backstage")
centred in the top bar and "Go live" as the only accent button at its
right. Same semantic colours as Connect, in both themes. The Studio adopts
the same feature-based layout (`apps/studio/src/features/*`) when it is
restyled; until then this section is the target, not a description of the
current Studio.
