﻿using Riok.Mapperly.Abstractions;
using Velopack.Deployment;
using Velopack.Packaging.Commands;
using Velopack.Packaging.Unix.Commands;
using Velopack.Packaging.Windows.Commands;
using Velopack.Vpk.Commands;

namespace Velopack.Vpk;

[Mapper(
    RequiredMappingStrategy = RequiredMappingStrategy.Target,
    EnabledConversions = MappingConversionType.None)]
public static partial class OptionMapper
{
    public static partial TDest Map<TDest>(object source);

    public static partial OsxPackOptions ToOptions(this OsxPackCommand cmd);
    public static partial WindowsPackOptions ToOptions(this WindowsPackCommand cmd);
    public static partial LinuxPackOptions ToOptions(this LinuxPackCommand cmd);
    public static partial OsxBundleOptions ToOptions(this OsxBundleCommand cmd);
    public static partial GitHubDownloadOptions ToOptions(this GitHubDownloadCommand cmd);
    public static partial GitHubUploadOptions ToOptions(this GitHubUploadCommand cmd);
    public static partial HttpDownloadOptions ToOptions(this HttpDownloadCommand cmd);
    public static partial LocalUploadOptions ToOptions(this LocalUploadCommand cmd);
    public static partial LocalDownloadOptions ToOptions(this LocalDownloadCommand cmd);
    public static partial S3DownloadOptions ToOptions(this S3DownloadCommand cmd);
    public static partial S3UploadOptions ToOptions(this S3UploadCommand cmd);
    public static partial DeltaGenOptions ToOptions(this DeltaGenCommand cmd);
    public static partial DeltaPatchOptions ToOptions(this DeltaPatchCommand cmd);

    private static DirectoryInfo StringToDirectoryInfo(string t)
    {
        var di = new DirectoryInfo(t);
        if (!di.Exists) di.Create();
        return di;
    }

    private static RID StringToRID(string t) => RID.Parse(t);
}
