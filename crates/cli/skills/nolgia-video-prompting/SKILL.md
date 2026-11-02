---
name: nolgia-video-prompting
description: "Directing craft for NOLGIA video generation: the film standard (references with stated jobs, casting and styling for the role, location plates, one structured prompt per take, written cuts at the genre's rhythm, best of N takes, sound, typeset words, the realism finish), plus the breathing pattern, per-shot sound design and the keep-what-works iteration loop. Use when: writing or fixing video prompts, planning a multi-clip production, a short film, a series episode, a trailer, a commercial or a music video on NOLGIA, or deciding which shots to regenerate. NOT for: platform mechanics such as auth, flags, jobs and credits (use nolgia-platform), vertical UGC ad pipelines (use nolgia-ugc-ads), or still-image prompting."
version: 1.11.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, video-prompting, directing, seedance, minimax-h3, multi-shot, filmmaking, realism]
  related_skills: [nolgia-platform, nolgia-ugc-ads]
---

# NOLGIA Video Prompting

Distilled from the source records of professionally made AI films (movies, short films, music
videos, commercials, ads and trailers: every prompt, reference and setting they sent) and from
NOLGIA's own realism lab, which proved what makes generated live action read as shot on a cinema
camera or a DSLR. Two layers: what to put in a prompt, and how to run a production without
burning credits.

## Prompt anatomy (one simple clip)

```
Photoreal. <SHOT SIZE>. <subject + one action>. <environment, time, weather>.
<camera: one focal length, the move>. <the light, as what it does>.
<one look line>. AUDIO: <sound design>, no music.
```

Example: *Photoreal. WIDE SHOT. A rust-flecked hatchback moves south on an
empty two-lane highway through flat farmland. Late afternoon, overcast gray
sky. Locked off at road height on a 40 mm lens. Soft overcast daylight from
above, no direct sun, wet asphalt a shade darker than the verge. A 35mm film
look, fine organic grain, natural true-to-life colour. AUDIO: car engine,
wind, distant birds, no music.*

Rules that survived production:
- **One action per shot.** Two actions and the model picks one or morphs.
- **Repeat exact wording** for wardrobe, props and locations in every prompt
  that shows them ("olive field jacket with a tear at the left elbow",
  verbatim, every time). Paraphrasing drifts.
- **Describe what is in the frame, at the size the distilled films write
  it**: the people (section "Casting and styling for the role"), the place as
  set objects, the first frame in words, the action on a timeline, one focal
  length, the move by name with its reason, the light as what it does (section
  "The LOOK block") and one short look line. The director of photography's
  spec (camera body and format, lens set, T-stop, stock number, hex codes, LUT)
  stays in your own shot plan and the finish, never in the prompt: a T-stop and
  a stock number sat in about 95 of our 141 film prompts and appear in under
  1.5 per 1,000 of the distilled films'. The gear behind the light (fixtures,
  modifiers, flags, frames, grip, stands) never goes in a prompt either
  (section "The gear stays out of the prompt").
- **Never a pile of buzzwords**: no "cinematic" as filler, no "8k, masterpiece,
  hyperrealistic, ultra detailed, award winning, perfect, beautiful bokeh,
  flawless, glass smooth". Those stacks gave a centred, symmetric advert frame
  with teal and orange. "Photoreal" once, in the opening line, is what the
  distilled films write (about 700 of every 1,000 people prompts).
- **Lens artifacts never go in a prompt; texture does, once** ("streak
  flares", "barrel distortion" and "vignette corners" render as overlays).
  Write "fine organic grain" once, in the one short look line ("A 35mm film
  look, fine organic grain, natural true-to-life colour."). Fine grain is the
  look term found most often behind takes that reached a final cut in the
  distilled films.
- **Never default to teal and orange**, neon, or any colour cast the LOOK block
  does not name (section "The LOOK block"): write the colours that are in the
  frame, never the casts you fear.
- **No text in generated frames.** Models garble it; typeset words in the edit.
  Printed props and gear (tickets, labels, dive masks, control panels) invite
  pseudo lettering: write them plain, keep them blank or out of focus.
- **Never name a scene object or piece of gear you do not want**, not even to
  deny it ("jacket free sleeves" put a jacket on her). Faults are not named
  either: no FORBIDDEN list of the faults you fear (plastic or airbrushed skin,
  glossy highlights, a generic AI face, model looks, makeup, beauty lighting,
  over-sharp HDR, a CGI look, morphing, extra fingers, duplicate people, heavy
  grain, glow, teal and orange, lens flares). The model reads fault words as
  content. A take ends on one CONTENT line of what is simply not in the scene
  or the soundtrack: "No on-screen text or subtitles. No music." plus a
  headcount where it matters ("Nobody else is in the room."). A camera ban that
  is a move ("no zoom") stays in the CAMERA line.
- **Three characters per shot at most**; close-up references beat distant ones.

## Films: the standard, not shot-by-shot prompts

A short film, series episode, trailer, commercial, skit or music video is never
made by sending each line of a shot list to `nolgia gen video`. That is how an
in-house short came out looking like AI (2026-10-31). The order that works:

1. **Story first**: logline, scene list, the script with every spoken line, the
   take plan. Ask once for the frame (21:9, 4:3, 16:9 or 9:16) and check it
   against the route (section "Frame and finish").
2. **Write the LOOK block**, then **cast, style, products and locations as
   stills** (below). Check every still at full size against the LOOK block before
   it becomes a reference.
3. **Write each take** as one structured prompt, as one of two take types, at the
   genre's rhythm.
4. **Shoot**: best of N on performance takes, draft cheap and promote, patch with
   video edit before a reroll.
5. **Edit** on the cuts the takes actually contain; lay the score or song; typeset
   every word.
6. **Finish and check**: the realism post chain, every cut judged alone against
   the script and the LOOK block; bits per pixel and palette spread are warnings.

### References, not keyframes

- **Every take carries 2 to 5 references, each with a stated job and what not to
  take from it** (`--element`, up to 9 on `seedance-2.5` and `minimax-h3`; the
  distilled films' median is 4): a character reference per person, an empty
  location plate, a product sheet where there is a product, and a voice
  reference per speaker (`--audio-ref`). "@Image1 is Mara: her face, hair and
  clothes only; not the grey backdrop, the panel layout or the pose." "@Image2
  is the harbour looking north: the space, geography and light direction only;
  nobody in it."
- **A written FIRST FRAME block** pins the opening picture: shot size, field of
  view, camera height and side, each person's position as x/y percent of the
  frame, which way they face, the action already in motion. The first frame is
  written, never handed over as one generated keyframe called "the opening
  frame".
- **A keyframe still is conditional**: only for a take with no references (a true
  start frame), a pose that must be exact, a stylized locked camera shot, or a
  hero frame whose composition must be exact. With references on `seedance-2.5`
  or `minimax-h3` it rides as an element with one job, composition only
  ("@Image3: composition only: match its framing and blocking, not its light,
  faces or texture"), since both refuse a start frame together with reference
  media.
  That is model specific: `kling-v4` takes a start frame beside elements, and
  `wan-3.0` takes a start frame only. Element images run 0.4 to 2.5 in aspect
  ratio.
- **Grok takes its start frame alone.** On `grok-imagine-video-1.5` send `--input`
  with no `--element`: with elements it ignores the start frame. Its prompt
  limit is 4,096 characters and it has no 21:9.
- **No `--character-id` for story roles.** A bound character carries its canonical
  outfit and accessories into every render and overrides the written wardrobe.
  Dress the character into the role as a cast sheet and send the sheet.

### Casting and styling for the role

- **Cast the role by what you write, never by a lock sentence**: the person by
  age as a number, job, build and height, hair, and the role's clothes by
  colour, fabric and wear; then pick the face that fits the role from the
  candidates. One face line per person:
  "DENISE, 52, a school bus driver, a Mexican-American woman, about 160 cm,
  solid build; a round face, dark brown eyes, shoulder-length dark brown hair
  with grey at the roots in a loose black clip; a navy fleece zip jacket over a
  grey work polo, a plain gold wedding band."
- **At most one specific mark per face**, and only when the story needs it (a
  boxer's healed brow scar, a farmer's sun-browned forearms); sweat or rain only
  when the scene makes it, said once. Never a catalogue of pores, lines, crow's
  feet, spots, moles, freckles, capillaries, shine or chapped lips: the model
  draws every listed flaw, literally and at once, and the face reads as made
  (ours averaged 3.0 flaws a face, the distilled films 0.77). No skin clause
  ("skin stays skin coloured", "it never turns smooth, waxy or plastic") and no
  identity lock ("not a model and not an actor", "if any instruction pulls
  toward a prettier face, drop it"): in a same-prompt test those lines alone
  turned a real face oily, older and posed.
- **Send `--aura` on every image call, never omit it**: omitted, the server adds
  its own skin and finish text to any prompt that reads as a person. Send
  `--aura false` for every still written this way (cast stills, sheets,
  keyframes) and every plate: the prompt already writes its light. A face
  reference works with `--aura false` (the identity lock and the face check
  still apply). Never write beauty words (flawless, poreless, glass smooth,
  lit from within, dewy, symmetrical).
- **Stills at 2k: make 4 candidates and pick 1** at full size (the distilled
  films made person stills at 2k 97.8% of the time, in batches of 4 on 69%).
  People stills on `nano-banana-pro --quality 2k` (21 credits each; it makes one
  image and takes one reference per job, so 4 candidates are 4 jobs);
  `seedream-v5-pro` only on a route that does not rewrite the prompt (its one
  route today always rewrites it); `gpt-image-2.5-flare` for words that must
  read (labels, signs, packaging) and product sheets from the real photos;
  empty plates on `nano-banana-2.1 --quality 2k` (5 credits); sheets and state
  edits on `nano-banana-pro` or Flare; never a flux model for people or
  commercial frames.

  ```bash
  nolgia gen image --model nano-banana-pro --quality 2k --aura false \
    --aspect-ratio 16:9 --prompt "$(cat denise-cast.txt)" --out denise-1.png
  # run it 4 times (denise-1..4.png) and keep the one that fits the role
  ```
- **One composite sheet per role**: one image on a plain mid grey backdrop, no labels: full
  body front and back with the heads masked, plus one large face panel, made by
  editing the approved cast still so the face holds. One face, one person per
  sheet. Never an expression grid as the identity reference.
- **Every state is an edit of the approved sheet**, saved as its own reference
  (wardrobe change, wet, injured, night): "this exact character sheet, but she is
  soaked through; change only the wetness".
- **Wardrobe and props follow the role and the scene**: would this person wear or
  carry this, here, today? Worn as the job makes them; "no makeup, no jewelry, no
  watch" unless the role has them. Style references guide taste; the styling must
  fit the piece.
- **Products get a sheet from the customer's real photos**: "one full front, one
  3/4 perspective and one closeup of all the label text of the product from
  @image1. No added text, just the 3 angles." Write the packaging phrase once and
  paste it word for word into every take, with a scale lock ("about two thirds of
  her face height") and a label lock ("the label faces camera and turns with the
  can; never redraw or re letter it").

### Locations

- **Empty plates, one per camera direction**; a reverse angle is its own plate;
  three quarter, never frontal.
- **Light and weather states are edits of the approved plate** (night, power cut,
  rain): a plate with the lamp on pulls the lamp on.
- **Browse cheap**: 6 to 8 cheap variants per location, keep the one whose light
  already tells the story. Dress it lived in, object by object; signs blank.

### The LOOK block (before any still)

Every film, commercial, music video and trailer has a LOOK block, written before
the first still, at the size the distilled films write it: four short lines,
not a spec sheet. Its lines go into every scene still and take of that lighting
setup, word for word, each in its place (the light in LIGHT, the focal length in
CAMERA, the look and colour lines near the end):

- **Light**: soft motivated natural light unless the story needs otherwise.
  "Soft natural light: window daylight from camera left (contre-jour where it
  fits), bouncing off the pale walls onto her face, a faint haze in the air.
  Faces readable; highlights roll off softly." A night or interior story writes
  the practical as a set object and what its light does ("lit only by the desk
  lamp beside him, its warm pool falling across his hands"); the story's own
  hard light (a boxing ring's top light) is written once, as its effect. Named
  setups stay allowed (low key or high key, motivated practicals, top light,
  contre-jour with the camera on the shadow side, Rembrandt, loop, split,
  butterfly, direct on-camera flash, dark-field for glass). Every light is
  written as its effect: where it comes from (from off-frame, unless it is a
  practical really in the shot), how soft it is, what it falls on, how it falls
  off. A plain "warm" or "cool" may sit beside a Kelvin number; never a hue mix
  word for light such as "green white", "yellow green" or "teal": they paint the
  frame. A fluorescent corrected with minus green is written as "4000 K, no
  green cast".
- **Optics**: one focal length, "shallow but readable depth of field, the
  background falling into a soft, natural blur". In a video take the field of
  view in degrees works as a lens lock and "waist up" as a shot size; on a still
  frame by shot size and set the shape by the aspect ratio parameter.
- **One look line**, with "fine organic grain" once: "A 35mm film look, fine
  organic grain, natural true-to-life colour."
- **Colour as one sentence** of the named colours in the frame, optionally a
  60:30:10 split in words: "mostly cream walls and worn pine, faded chambray
  blue second, the amber honey as the accent." No hex codes in a still or take.
  A coloured light (neon, a red practical) is in it only when the story calls
  for it, written as a set object.

The director of photography's spec stays in your shot plan: the camera body and
format, the lens set and T-stop, the stock and its number, the hex codes and
ratios, the LUT and the crew notes. None of it is pasted into a prompt. The stock
you plan picks the LUT the finish applies.

A new lighting setup gets its own LOOK block. Palette spread and bits per pixel
are warnings, never targets: never add a coloured source or change the light to
raise a number. A still or take that fails is judged against the LOOK block and
regenerated from it. No colour fix in post as a plan: the look comes from the
written stills and takes, and the grade only finishes.

### The gear stays out of the prompt

The models draw the gear a prompt names. In NOLGIA's live-action test
(2026-11-01) a book light came back as a reading lamp in the shot, a China ball
as a paper lantern, an apple box as a box the actor sat on and an egg crate as
an egg carton; a named octabox and a black flag for negative fill each stood in
the frame in 2 of 2 stills, and "every light is off frame" cleared only 1 of 2.
The same prompts with the light written as its effect drew no gear in 4 of 4.
The professional AI productions we studied agree: fixture names almost never
describe a light, and none of their kept takes names an off-frame fixture.

- In any still or take prompt, never name the gear behind the light: fixtures,
  modifiers, flags, frames, grip or stands (Fresnel, HMI, Kino Flo, SkyPanel,
  Astera, China ball, book light, octabox, softbox, strip box, beauty dish,
  black flag, negative fill, bounce board, reflector, diffusion frame, apple
  box, egg crate, C-stand, gels), and never as a setup name either.
- Write the effect: the side and height it comes from, from off-frame, its size
  and softness, its Kelvin, what it falls on, how it falls off and the shadow it
  leaves. "A 1.5 m octabox camera left" becomes "a large, soft key at 5500 K
  from off-frame, camera left, wrapping the near cheek, a soft-edged shadow on
  the far one"; "a black flag camera left" becomes "the camera-left side of the
  room is dark and nothing bounces light back"; "a China ball overhead" becomes
  "a soft, round glow from above, from off-frame, falling off fast down the
  walls". Direction, rim light, falloff, the time of day, window light and
  Kelvin numbers carry the look.
- A studio, sheet, product or plate still, where an empty backdrop is the whole
  scene, gets one ban line that names categories only: "no lighting equipment
  anywhere in the frame: no light stands, lamps or fixtures; a clean, empty
  backdrop". Never a list of specific fixtures.
- Gear meant to be seen (a behind-the-scenes shot, a photo shoot, a creator's
  ring light) is named as a set object in the shot.
- Keep naming what names a look or something really in the shot: the setups
  (Rembrandt, butterfly, low key, contre-jour, motivated practicals, direct
  on-camera flash, dark-field), one focal length, the camera moves (when a move
  implies a rig the frame could show, add "no camera rig, dolly track, rails or
  crew visible"), and practicals in the shot, written as set objects. The camera
  body, lens set, T-stop and stock number stay in the shot plan (section "The
  LOOK block").
- The gear names stay in your own shot plan and the LOOK block's crew notes,
  which are never pasted into a prompt. Check the first still for anything the
  light words drew (a lamp, a lantern, a stand, a black panel) and remake it
  before any video credits.

### One structured prompt per take, in a held order

The distilled films' order: SCENE CONTEXT (who is where and what happens now,
opening with "Photoreal"), ACTIVE REFERENCES (each with its job), CHARACTER(S)
(the face line per person, the product phrase), LOCATION (the place as set
objects), FIRST FRAME, FORMAT (the cuts), CAMERA (the one focal length with its
depth of field, then one move per segment with its reason, led by the ones the
distilled films keep: handheld written as an operator with breathing and one late
correcting reframe, locked off, a slow push in, tracking, crane, rack focus; the
camera holds through lines; a zoom, whip pan, speed ramp, orbit or gimbal float
only when the shot asks, and "no zoom" stays here), ACTION TIMING, DIALOGUE and
VOICE, ACTING, PHYSICS, LIGHT (the LOOK block's light, each light as its effect,
never the gear by name; only sources the LOOK block names), AUDIO ("diegetic
sound only, no music"), the LOOK line and the colour sentence, then the CONTENT
line ("No on-screen text or subtitles. No music." plus a headcount). No IDENTITY
or LOCKS block and no FORBIDDEN list of faults (section "Prompt anatomy").

- **ACTING**: the full block for a performance take (a scene direction, then per
  character motive, goal, obstacle, tactic, beats in seconds, END STATE; "do not
  play the emotion"); a light form for comedy, action and product takes (physical
  beats per segment plus an END STATE).
- **One LOOK block per film and per lighting setup**, pasted identically into
  every still and take, then a contradiction check: a night interior never
  inherits a daylight block, and no sentence in the take names a colour or a
  light the LOOK block does not.
- **A people take stays at or under about 8,000 characters** (the distilled
  films' median is 8,001; our 13,000 to 18,000 character takes carried the
  boilerplate this skill now leaves out), and always under the model's own
  limit, `video.max_prompt_chars` on `nolgia models get <model> --json`
  (`seedance-2.5` 19,800, `minimax-h3` 7,000, `grok-imagine-video-1.5` 4,096).
  The long structured prompt is proven on `seedance-2.5` only; `kling-v4` and
  Wan get lean prompts (the distilled Kling
  B roll ran about 260 characters, and the vendor rewrites every Wan prompt).
  `seedance-2.5` reads timestamps in steps of about a second; the Seedance 2.0
  ids ignore them and follow "Shot 1", "Shot 2" beats. Over the limit the request is refused up front; trim
  style prose first, never a reference job, the acting or a line. A CLI build older than
  the per-model limits (0.2.35 among them) refuses anything over 4,000
  characters client side ("longer than 4000 characters"): send that take with
  `nolgia api POST /generate/video` and a JSON body instead.

### Two take types, at the genre's rhythm

- **Coverage takes, 12 to 30 s, with 3 to 6 written cuts**: "One continuous
  sequence, FOUR hard-cut segments, 14 s of picture. HARD CUT at 3.5, 7.0 and
  10.5 s." then one line per segment, each a new shot size, cuts between spoken
  lines. `seedance-2.5` runs to 30 s; `minimax-h3` and Grok to 15 s.
- **Single shots, 4 to 8 s**, for action, inserts, VFX and the product hero.
- **The multi-shot flag stays off** (no `--shot`): the cuts are written in the prompt.
  `kling-v4` has no such flag and splits a take itself, so a single shot on it
  says "one continuous long take without any cuts, no subtitles, no background
  music".
- **Write what is in the frame on every lane**; the one negative line is the
  CONTENT line (no on-screen text or subtitles, no music, the headcount). Only
  Veo 3.1 and Omni take `--negative-prompt` (`video.negative_prompt_supported`);
  every other model accepts it, sends nothing and says so in the job's
  `warnings`, so what must stay out lives in the prompt text, as content, never
  as a list of faults.
- **Handles**: set the duration 1 to 2 s longer than the written timeline and
  write the final hold as the END STATE.
- **Rhythm**: dialogue drama 1 to 3 segments of 4 to 7.5 s (it holds); comedy and
  action 3 to 5 of 3 to 4 s; fights 9 to 15 of about 1 s in a 15 s take; music
  video story takes about 3 s a cut; commercials and ads a cut every 1.8 to 2 s.
- **Cut the edit on the cuts the take actually contains**, never on the written
  seconds (they drift by up to 1.7 s). Every take ends on its END STATE and the
  next starts from it.

### Takes

- **Best of N by re running the identical prompt**: 3 for a hero or performance
  take, keep the one that plays the script; 1 plus review for inserts and wides.
- **Draft cheap, then promote**: prove a hard take's prompt at `seedance-2.5`
  480p, then render the identical prompt at 720p (no seed on that lane, so the
  720p render is a new roll of a proven prompt).
- **Patch with video edit before re rolling**: `seedance-2.5` with `video_task`
  `edit` keeps everything and changes one named thing.
- **Alternates of the same beat may intercut** at detected cuts; a scene is never
  assembled from unrelated single generations.

### Sound

- **Takes say "no music"**; the score or song is laid in the edit. Music goes in a
  take only when it plays in the scene (a radio).
- **People speaking go on `seedance-2.5` at 720p** (21:9 for films), with their
  character references riding as elements. In the speech lab both lanes said 10 of 10 words,
  but Seedance with voice references matched each voice better, articulated more
  and kept the skin real (0.28 bits per pixel against H3's 0.048).
  - **One voice audio reference per speaker**: a 5 to 8 s line on
    `elevenlabs-tts-v3` (about 2 credits) in the character's register, using a
    different sentence from the script, attached with `--audio-ref` in speaker
    order, each with a job: "[Audio 1] is the COOK's one and only voice: match
    this timbre; never play it, never repeat its words."
  - **A VOICE LOCK**: who speaks in which voice, the voices never swap, nobody else
    has a voice.
  - **THE SCRIPT**: "only these N lines, word for word, in this order, one mouth at
    a time; nobody else speaks, hums or mouths words", each line in a timed window
    with a delivery note, a pause, and the camera held still for the lines.
  - **Cut on the detected speech**, not the written times: Seedance spoke about
    0.8 s late.
  - Spell tricky words phonetically (hyphenated syllables, stress in capitals); a
    line in another language is written in its native words with the translation
    in parentheses.
- **Sung performance** (a music video's sung close up or medium close up only) goes
  on `minimax-h3` at 768p: the song slice is the audio reference (`--audio-ref`),
  the take is as long as the slice, the singer's close-up still rides as element
  @Image1 (composition only) beside the cast sheet, the face
  fills at least a third of the frame with the lips unobstructed, one continuous
  move per phrase. H3 closed all 5 M and B sounds within a frame and returns the
  real slice, so it drops in at 0. Everything around it (story, B roll, wides) stays
  on `seedance-2.5` 720p with the song laid in the edit. Seedance sealed 2 of 5 and
  re-renders the song 170 ms early: never lay that re-render in the edit.
- **Translation subtitles are typeset** when a line is not in the audience's
  language; a film still gets no captions.

### Words on screen and titles

- **Every word is typeset in the edit and checked word for word** against the
  customer's text. The packshot leaves two thirds of the frame empty for type;
  words get a 2 s locked hold; end cards run 2.6 to 5 s.
- **Titles stay locked to the frame** unless match moved to a verified track; light
  effects animate light only; a credit roll moves a constant whole number of
  pixels per frame (4 to 6 px at 24 fps for 1080) and goes on after grain.

### Frame and finish

| Frame | `seedance-2.5` | `minimax-h3` | `grok-imagine-video-1.5` |
|---|---|---|---|
| 21:9 | 480p and 720p (720p renders 1470x630; 1080p refused) | 768p (1568x672) | no: compose 16:9 for the crop |
| 4:3 | 480p and 720p | yes | yes |
| 16:9, 9:16 | 480p, 720p, 1080p | yes | yes |

People stills are on `nano-banana-pro`, which renders 21:9; `seedream-v5-pro` has
no 21:9. Topaz on a 21:9 take
is untested: treat that upscale as open. Re-read `nolgia models get <model>` before
quoting a frame.

- **The live action lane is `seedance-2.5` at 720p with references**, for dialogue
  too. `minimax-h3` at 768p or 2k delivers 0.04 to 0.07 bits per pixel, which
  smears skin into wax: on a photoreal person use it only for a sung lip sync shot,
  never for dialogue; a painted or stylized character may use it for a hard
  performance shot. Avoid `veo-3.1-fast` for people who must read as real.
- **The cheaper lanes, offered as options, never forced** (5 s at 720p: `wan-3.0`
  28 credits, `kling-v4` 44, `seedance-2.5` 66; check `nolgia models get`).
  `kling-v4` is the budget lane for silent B roll, establishing shots, atmosphere
  and static photographic wides: image to video from a finished keyframe, 4 to 8 s,
  one move and one action, ending "single continuous shot; everything else in the
  frame stays still"; never speech, written cuts, fast contact action, VFX wides or
  hero close ups. Its audio is always on: send `strip_audio: true` in the API body
  for a silent file. `wan-3.0` is not routed yet for its one corpus job: the
  distilled films used Wan (2.7) for a stylized character speaking a supplied
  voice track, and our `wan-3.0` takes a start frame only, with no audio
  reference. For silent B roll it may be offered as the cheapest option, never the
  proven one: a lean prompt ending "No dialogue.", `strip_audio: true`, and a
  conform in the edit (Wan renders 30 fps, Seedance and Kling 24). Everything else, every
  photoreal person speaking included, stays on `seedance-2.5`.
- **Wides with several people**: make the wide keyframe as an edit of the closer
  keyframe, so the leads' faces come from a still where they are large; faces at 35
  to 45 px wide hold on `seedance-2.5` 720p.
- **Long takes**: 15 s with 3 or 4 written cuts landed within 0.25 s; give every
  new angle its own reference (a plate or keyframe edit at that angle): a reverse
  with none drifted.
- **Judge every still against the LOOK block before any video credits**: a take
  inherits its stills' colour. A still that drifts (a cast, a source the block does
  not name) is regenerated from the LOOK block's words, never patched by adding a
  light.
- **Check every still, and the first take, at full size for spec words rendering
  as on screen artifacts**: streaks, flares drawn as objects, rounded or vignetted
  corners, film borders, sprocket holes, letterbox bars, timecode or camera UI. If
  any appear, take the artifact words out and remake the still before any video
  credits (an in-house short came back with streaks and rounded corners).
- **The realism post chain, in this order**: keep the take at 720p; Topaz to 1080p
  with `topaz-proteus-natural` (split long takes at their cuts, since the restore
  window is about 7.4 s); grade with the LUT that matches the stock in your shot
  plan at half strength or less (`kodak-vision3-250d` day, `kodak-vision3-500t` tungsten
  or sodium night; `nolgia color-presets list` for the rest); film
  emulation (halation, slight softness, grain) LAST at 0.6 strength, at delivery
  resolution, never before the upscale, lighter when the take already carries
  fine grain. The `kodak-2383-print` finish is not the
  default: at half strength it turns shadows cyan and crushes blacks. The grade
  only finishes: it never rescues a colour the stills and takes got wrong.
- **Automatic checks are warnings, never targets**, on every raw take and the
  master: bits per pixel (bitrate divided by width x height x fps; under 0.12 warns
  of a starved stream) and palette spread (hue entropy under 2.0 bits over 36 bins,
  or a teal share over 0.40, warns of an unplanned cast). A warning is judged
  against the LOOK block: a narrow palette the block asks for passes. A skin
  texture number is no gate; judge skin by eye at full size.
- **Never**: `minimax-h3` at any resolution for a photoreal shot that does not
  need song lip sync; `kling-v4` for speech or a hero close up; a zoom, whip pan,
  speed ramp, orbit or gimbal float the shot did not ask for; dialogue routed to H3 just because it speaks; a start frame together with
  an audio reference (refused: the keyframe rides as an element); Seedance's
  re-rendered song laid in the edit; a new angle with no reference at that angle;
  a coloured source or a light change added to raise a number; a hue mix word for
  light ("green white", "teal"); a cast left for the grade to fix.

## The breathing pattern (multi-shot sequencing)

Every cut changes shot size: WIDE (establish), MEDIUM (observe), CLOSE-UP
(feel), WIDE (release). Never two adjacent shots at the same distance. After
every discovery, a half-second reaction beat on the protagonist: reaction shots
ARE the story. Never cut directly into a key moment: show approach, threshold,
reaction, event.

```bash
nolgia gen video --model seedance-2.5 --quality 720p --duration-seconds 14 \
  --generate-audio true --element <cast sheet uuid> --element <plate uuid> \
  --prompt "$(cat take-03.txt)"   # FORMAT: FOUR hard-cut segments, 12 s of picture,
                                  # HARD CUT at 3.0, 6.5, 9.0 s; WIDE, MEDIUM, CU, WIDE
```

## Sound is written into the shot

Every shot carries an AUDIO direction: the diegetic sounds by second and "no
music". Silence is a direction too ("total silence" is the strongest cut in the
toolbox). Audio as an afterthought produces cuts that feel aggressive and
unmotivated.

## The iteration loop (protects credits and quality)

1. Draft cheap: the lane's lowest tier (`seedance-2.5` 480p for a film take;
   compare prices with `nolgia models list --modality video` or `--cost-only`).
2. Review each clip at full size against the script: **KEEP / FIX / CUT**, written
   down.
3. **Never regenerate a KEEP.** Fix a FIX shot by video edit first, or regenerate
   it changing one variable at a time (a prompt sentence or a reference, not both).
4. On models that take `--seed` (`nolgia models get <model>`), reuse it when
   re-rolling composition so only your change moves.
5. Final pass on the keeper tier only for shots that earned it.

The classic failure: regenerating everything each round because feedback touched
some shots. Approved clips are assets: edit around them.

## Assemble the finished cut

Individual clips are not a film. Once the shots are approved, sequence them into
one rendered video with a Studio composition (see the `nolgia-platform` skill):
`nolgia compositions create --name <n> --clip <shot1> --clip <shot2> --render
--wait`. Clips play in the order given, each clip's audio is kept (`--mute` to drop
it). Do not report the production done until that render succeeds and you can
name the finished asset: a project folder full of clips is not the deliverable.
