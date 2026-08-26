import os
from PIL import Image, ImageDraw, ImageFont

img_dir = "integrations/streamdeck/com.trackhelm.controller.sdPlugin/images"
os.makedirs(img_dir, exist_ok=True)

actions = {
    "plugin_icon": ("☸", "#007aff", "#121418"),
    "category": ("☸", "#007aff", "#121418"),
    "playpause": ("▶ ⏸", "#30d158", "#121418"),
    "playpause_active": ("▶", "#30d158", "#1c2b1e"),
    "rewind": ("⏮", "#ff9f0a", "#121418"),
    "nexttrack": ("⏭", "#64d2ff", "#121418"),
    "prevtrack": ("⏮", "#64d2ff", "#121418"),
    "nextmarker": ("⇥", "#bf5af2", "#121418"),
    "prevmarker": ("⇤", "#bf5af2", "#121418"),
    "addmarker": ("+⚐", "#ffcc00", "#121418"),
    "pitchup": ("♯ +1", "#5e5ce6", "#121418"),
    "pitchdown": ("♭ -1", "#5e5ce6", "#121418"),
    "volup": ("🔊 +", "#0a84ff", "#121418"),
    "voldown": ("🔉 -", "#0a84ff", "#121418"),
    "speedup": ("⚡ +", "#ff9f0a", "#121418"),
    "speeddown": ("🐢 -", "#ff9f0a", "#121418"),
    "songinfo": ("🎵 INFO", "#64d2ff", "#121418"),
    "markerinfo": ("⚐ MARK", "#ffcc00", "#121418"),
    "loop": ("🔁", "#30d158", "#121418"),
    "cut": ("✂️", "#ff453a", "#121418"),
}

for name, (symbol, color, bg) in actions.items():
    # Generate @2x (144x144)
    size = 144
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    
    # Rounded card background
    draw.rounded_rectangle([4, 4, size - 5, size - 5], radius=24, fill=bg, outline="#2a2d36", width=4)
    
    # Text/Symbol
    try:
        font = ImageFont.truetype("/System/Library/Fonts/AppleSDGothicNeo.ttc", 44)
    except:
        font = ImageFont.load_default()
        
    bbox = draw.textbbox((0, 0), symbol, font=font)
    w = bbox[2] - bbox[0]
    h = bbox[3] - bbox[1]
    x = (size - w) / 2 - bbox[0]
    y = (size - h) / 2 - bbox[1]
    draw.text((x, y), symbol, font=font, fill=color)
    
    img.save(os.path.join(img_dir, f"{name}.png"))
    img.save(os.path.join(img_dir, f"{name}@2x.png"))
    
    # 72x72
    img72 = img.resize((72, 72), Image.Resampling.LANCZOS)
    img72.save(os.path.join(img_dir, f"{name}@1x.png"))

print("Generated all Stream Deck action icons.")
