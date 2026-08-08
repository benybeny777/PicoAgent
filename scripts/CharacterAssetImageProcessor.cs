using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;

namespace PicoAgent.Tools
{
    public sealed class ImageAnalysis
    {
        public int Width { get; set; }
        public int Height { get; set; }
        public int MinX { get; set; }
        public int MinY { get; set; }
        public int MaxX { get; set; }
        public int MaxY { get; set; }
        public long VisiblePixels { get; set; }
        public long SemiTransparentPixels { get; set; }
        public long LightNeutralEdgePixels { get; set; }
        public long DarkNeutralEdgePixels { get; set; }
        public int[] CornerAlpha { get; set; }
    }

    public static class CharacterAssetImageProcessor
    {
        public static void NormalizeAlpha(string inputPath, string outputPath, int threshold)
        {
            using (var loaded = new Bitmap(inputPath))
            using (var bitmap = loaded.Clone(
                new Rectangle(0, 0, loaded.Width, loaded.Height),
                PixelFormat.Format32bppArgb))
            {
                Mutate(bitmap, (bytes, offset) =>
                {
                    if (bytes[offset + 3] <= threshold)
                    {
                        bytes[offset] = 0;
                        bytes[offset + 1] = 0;
                        bytes[offset + 2] = 0;
                        bytes[offset + 3] = 0;
                    }
                });
                Directory.CreateDirectory(Path.GetDirectoryName(outputPath));
                bitmap.Save(outputPath, ImageFormat.Png);
            }
        }

        public static void RemoveWhiteMatte(string inputPath, string outputPath)
        {
            using (var loaded = new Bitmap(inputPath))
            using (var bitmap = loaded.Clone(
                new Rectangle(0, 0, loaded.Width, loaded.Height),
                PixelFormat.Format32bppArgb))
            {
                Mutate(bitmap, (bytes, offset) =>
                {
                    int alpha = bytes[offset + 3];
                    if (alpha <= 0 || alpha >= 255) return;
                    for (int channel = 0; channel < 3; channel++)
                    {
                        int value = 255 * (bytes[offset + channel] - (255 - alpha)) / alpha;
                        bytes[offset + channel] = (byte)Math.Max(0, Math.Min(255, value));
                    }
                });
                Directory.CreateDirectory(Path.GetDirectoryName(outputPath));
                bitmap.Save(outputPath, ImageFormat.Png);
            }
        }

        public static ImageAnalysis Analyze(string inputPath)
        {
            using (var loaded = new Bitmap(inputPath))
            using (var bitmap = loaded.Clone(
                new Rectangle(0, 0, loaded.Width, loaded.Height),
                PixelFormat.Format32bppArgb))
            {
                var result = new ImageAnalysis
                {
                    Width = bitmap.Width,
                    Height = bitmap.Height,
                    MinX = bitmap.Width,
                    MinY = bitmap.Height,
                    MaxX = -1,
                    MaxY = -1,
                    CornerAlpha = new int[]
                    {
                        bitmap.GetPixel(0, 0).A,
                        bitmap.GetPixel(bitmap.Width - 1, 0).A,
                        bitmap.GetPixel(0, bitmap.Height - 1).A,
                        bitmap.GetPixel(bitmap.Width - 1, bitmap.Height - 1).A,
                    },
                };
                Inspect(bitmap, (bytes, offset, x, y) =>
                {
                    int alpha = bytes[offset + 3];
                    if (alpha > 8)
                    {
                        result.VisiblePixels++;
                        if (x < result.MinX) result.MinX = x;
                        if (x > result.MaxX) result.MaxX = x;
                        if (y < result.MinY) result.MinY = y;
                        if (y > result.MaxY) result.MaxY = y;
                    }
                    if (alpha > 0 && alpha < 255)
                    {
                        result.SemiTransparentPixels++;
                        int blue = bytes[offset];
                        int green = bytes[offset + 1];
                        int red = bytes[offset + 2];
                        int maximum = Math.Max(red, Math.Max(green, blue));
                        int minimum = Math.Min(red, Math.Min(green, blue));
                        if (maximum - minimum < 24 && minimum > 224) result.LightNeutralEdgePixels++;
                        if (maximum - minimum < 24 && maximum < 32) result.DarkNeutralEdgePixels++;
                    }
                });
                return result;
            }
        }

        public static void ApplyScaledAlphaMask(
            string colorPath,
            string maskPath,
            string outputPath,
            int threshold)
        {
            using (var colorLoaded = new Bitmap(colorPath))
            using (var maskLoaded = new Bitmap(maskPath))
            using (var color = colorLoaded.Clone(
                new Rectangle(0, 0, colorLoaded.Width, colorLoaded.Height),
                PixelFormat.Format32bppArgb))
            using (var mask = new Bitmap(colorLoaded.Width, colorLoaded.Height, PixelFormat.Format32bppArgb))
            {
                using (var graphics = Graphics.FromImage(mask))
                using (var attributes = new ImageAttributes())
                {
                    graphics.CompositingMode = System.Drawing.Drawing2D.CompositingMode.SourceCopy;
                    graphics.CompositingQuality = System.Drawing.Drawing2D.CompositingQuality.HighQuality;
                    graphics.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
                    graphics.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
                    attributes.SetWrapMode(System.Drawing.Drawing2D.WrapMode.TileFlipXY);
                    graphics.DrawImage(
                        maskLoaded,
                        new Rectangle(0, 0, mask.Width, mask.Height),
                        0,
                        0,
                        maskLoaded.Width,
                        maskLoaded.Height,
                        GraphicsUnit.Pixel,
                        attributes);
                }
                ApplyAlpha(color, mask, threshold);
                Directory.CreateDirectory(Path.GetDirectoryName(outputPath));
                color.Save(outputPath, ImageFormat.Png);
            }
        }

        private static void Mutate(Bitmap bitmap, Action<byte[], int> visitor)
        {
            var rectangle = new Rectangle(0, 0, bitmap.Width, bitmap.Height);
            var data = bitmap.LockBits(rectangle, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
            try
            {
                int stride = Math.Abs(data.Stride);
                var bytes = new byte[stride * bitmap.Height];
                Marshal.Copy(data.Scan0, bytes, 0, bytes.Length);
                for (int y = 0; y < bitmap.Height; y++)
                {
                    for (int x = 0; x < bitmap.Width; x++)
                    {
                        visitor(bytes, y * stride + x * 4);
                    }
                }
                Marshal.Copy(bytes, 0, data.Scan0, bytes.Length);
            }
            finally
            {
                bitmap.UnlockBits(data);
            }
        }

        private static void Inspect(Bitmap bitmap, Action<byte[], int, int, int> visitor)
        {
            var rectangle = new Rectangle(0, 0, bitmap.Width, bitmap.Height);
            var data = bitmap.LockBits(rectangle, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try
            {
                int stride = Math.Abs(data.Stride);
                var bytes = new byte[stride * bitmap.Height];
                Marshal.Copy(data.Scan0, bytes, 0, bytes.Length);
                for (int y = 0; y < bitmap.Height; y++)
                {
                    for (int x = 0; x < bitmap.Width; x++)
                    {
                        visitor(bytes, y * stride + x * 4, x, y);
                    }
                }
            }
            finally
            {
                bitmap.UnlockBits(data);
            }
        }

        private static void ApplyAlpha(Bitmap color, Bitmap mask, int threshold)
        {
            var rectangle = new Rectangle(0, 0, color.Width, color.Height);
            var colorData = color.LockBits(rectangle, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
            var maskData = mask.LockBits(rectangle, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try
            {
                int colorStride = Math.Abs(colorData.Stride);
                int maskStride = Math.Abs(maskData.Stride);
                var colorBytes = new byte[colorStride * color.Height];
                var maskBytes = new byte[maskStride * mask.Height];
                Marshal.Copy(colorData.Scan0, colorBytes, 0, colorBytes.Length);
                Marshal.Copy(maskData.Scan0, maskBytes, 0, maskBytes.Length);
                for (int y = 0; y < color.Height; y++)
                {
                    for (int x = 0; x < color.Width; x++)
                    {
                        int colorOffset = y * colorStride + x * 4;
                        int maskOffset = y * maskStride + x * 4;
                        int alpha = maskBytes[maskOffset + 3];
                        if (alpha <= threshold)
                        {
                            colorBytes[colorOffset] = 0;
                            colorBytes[colorOffset + 1] = 0;
                            colorBytes[colorOffset + 2] = 0;
                            colorBytes[colorOffset + 3] = 0;
                        }
                        else
                        {
                            colorBytes[colorOffset + 3] = (byte)alpha;
                        }
                    }
                }
                Marshal.Copy(colorBytes, 0, colorData.Scan0, colorBytes.Length);
            }
            finally
            {
                try
                {
                    mask.UnlockBits(maskData);
                }
                finally
                {
                    color.UnlockBits(colorData);
                }
            }
        }
    }
}
