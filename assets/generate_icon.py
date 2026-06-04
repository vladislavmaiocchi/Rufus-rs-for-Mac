import sys
from PIL import Image, ImageDraw, ImageOps

def create_mac_icon_from_source(source_path, size):
    # 1. Create a transparent canvas
    canvas = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    s = size / 1024.0
    
    # 2. Draw a white/light-grey squircle (standard macOS background)
    # Most Mac apps have a white or very light background for the squircle
    margin = 40 * s
    draw.rounded_rectangle(
        [margin, margin, size - margin, size - margin],
        radius=180 * s,
        fill=(255, 255, 255, 255),
        outline=(220, 220, 225, 255),
        width=int(2 * s)
    )
    
    # 3. Open and process the source Rufus icon
    try:
        source = Image.open(source_path).convert("RGBA")
        
        # Calculate size for the source image inside the squircle
        # We want it to be large but centered
        inner_margin = 150 * s
        target_size = int(size - inner_margin * 2)
        
        # Resize source while maintaining aspect ratio
        source.thumbnail((target_size, target_size), Image.Resampling.LANCZOS)
        
        # Center the source image
        pos_x = int((size - source.width) / 2)
        pos_y = int((size - source.height) / 2)
        
        canvas.alpha_composite(source, (pos_x, pos_y))
    except Exception as e:
        print(f"Error processing source image: {e}")
        return None

    return canvas

source_img = "/Users/vladik/Downloads/rufus-icon.webp"
sizes = [16, 32, 64, 128, 256, 512, 1024]
for sz in sizes:
    icon = create_mac_icon_from_source(source_img, sz)
    if icon:
        icon.save(f"rufus-rs/assets/icon.iconset/icon_{sz}x{sz}.png")
        if sz <= 512:
            icon2x = create_mac_icon_from_source(source_img, sz * 2)
            if icon2x:
                icon2x.save(f"rufus-rs/assets/icon.iconset/icon_{sz}x{sz}@2x.png")
