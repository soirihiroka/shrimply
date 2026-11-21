# Shrimply UI Design Guidelines

## Philosophy

Shrimply aims to be simple yet powerful.

Don't make Shrimply look or behave like other video editors just because that's what users have come to expect. 
Every design decision should have a purpose.

Above all, make Shrimply enjoyable to use.

But for the dimwit who can't think:  
I've been an iMovie user for a long time, and I still believe it got the fundamentals of video editor UI right.  
If it is unclear how the UI should look, imagine how iMovie would do it.

## Design Principles


TLDR: Keep what matters visible, make everything else accessible, and never add complexity without a good reason.

Although Shrimply may resemble a GNOME application, it doesn't strictly follow the GNOME Human Interface Guidelines (HIG). Instead, use the following questions to guide UI decisions.



### 1. Does this interaction need to exist?

- No: Don't add it.
- Yes: Does it need to be visible?
  - Yes: Make it part of the UI.
  - No: Consider a keyboard shortcut or gesture instead.

### 2. Is the UI getting too crowded?

- Does everything fit comfortably on a 1080p display?
- Is the interface becoming visually overwhelming?
- If so, look for opportunities to simplify or fold less frequently used controls.

### 3. Can it be an icon button?

If an action can be clearly represented by an icon, prefer an icon button over a text button.

### 4. Is the behavior obvious?

- Yes: Don't add unnecessary documentation.
- No: Explain it from the user's perspective.
  - Did you realize you just made a horrible UX design?
    - Yes: Change it to what a reasonable person would expect.
  - Focus on what it does, not how it's implemented.
  - Prioritize behaviors that might differ from what users naturally expect.
  - Avoid implementation details and obscure edge cases unless they're relevant to the user.
