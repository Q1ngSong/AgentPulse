// 按 bundle id 读本机某个 App 的图标，编成 PNG 交给 Rust。跟 notifications/macos.m 一样是薄桥接层。
#import <AppKit/AppKit.h>

// 成功返回 1 并把一段 malloc 出来的 PNG 数据交给调用方（用 ap_icon_free 释放）；
// 找不到这个 bundle id 对应的 App、或者渲染失败，返回 0，不写 out_data/out_len。
int ap_icon_for_bundle(const char *bundle_id, unsigned char **out_data, size_t *out_len) {
    @autoreleasepool {
        NSString *bid = [NSString stringWithUTF8String:bundle_id];
        NSString *path = [NSWorkspace.sharedWorkspace absolutePathForAppBundleWithIdentifier:bid];
        if (!path) return 0;
        NSImage *icon = [NSWorkspace.sharedWorkspace iconForFile:path];
        if (!icon) return 0;

        // 悬浮窗里的小图标，128x128 足够清晰，不用原图那么大的尺寸
        NSSize size = NSMakeSize(128, 128);
        NSImage *resized = [[NSImage alloc] initWithSize:size];
        [resized lockFocus];
        [icon drawInRect:NSMakeRect(0, 0, size.width, size.height)
                 fromRect:NSZeroRect operation:NSCompositingOperationSourceOver fraction:1.0];
        [resized unlockFocus];

        NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:[resized TIFFRepresentation]];
        NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        if (!png || !png.length) return 0;

        unsigned char *buf = malloc(png.length);
        if (!buf) return 0;
        memcpy(buf, png.bytes, png.length);
        *out_data = buf;
        *out_len = png.length;
        return 1;
    }
}

void ap_icon_free(unsigned char *data) {
    free(data);
}
