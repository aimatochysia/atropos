import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

/// Picking files and folders as real filesystem paths, because the Rust core
/// reads and writes paths, never Android content:// URIs.
class PlatformFiles {
  const PlatformFiles();

  static const _storage = MethodChannel('atropos/storage');

  /// Folder the app keeps its own settings in.
  Future<String> configDir() async =>
      (await getApplicationSupportDirectory()).path;

  /// Where new vaults go unless the person picks somewhere else.
  Future<String> defaultVaultParent() async {
    if (Platform.isAndroid) return '/storage/emulated/0/Atropos';
    final docs = await getApplicationDocumentsDirectory();
    return '${docs.path}${Platform.pathSeparator}Atropos';
  }

  /// Android needs "All files access" before vaults can live in normal
  /// folders. Always true elsewhere.
  Future<bool> hasStorageAccess() async {
    if (!Platform.isAndroid) return true;
    return await _storage.invokeMethod<bool>('hasAllFilesAccess') ?? false;
  }

  Future<void> requestStorageAccess() =>
      _storage.invokeMethod<void>('requestAllFilesAccess');

  /// A folder on disk, or null when cancelled.
  Future<String?> pickFolder({String? title}) async {
    if (Platform.isAndroid) {
      return _storage.invokeMethod<String>('pickDirectory');
    }
    return getDirectoryPath(confirmButtonText: title ?? 'Choose');
  }

  /// Files on disk. On Android these are temporary copies the picker made
  /// ([PickedFiles.temporary]) and must be deleted after use.
  Future<PickedFiles?> pickFiles() async {
    final files = await openFiles();
    if (files.isEmpty) return null;
    return PickedFiles(
      files.map((f) => f.path).toList(),
      temporary: Platform.isAndroid,
    );
  }

  /// One file to open in place, such as an `.atropos` package that may be
  /// changed. On Android this is the real file when the picker can say
  /// where it is, and otherwise a temporary copy.
  Future<PickedFiles?> pickFileInPlace() async {
    if (Platform.isAndroid && await hasStorageAccess()) {
      final path = await _storage.invokeMethod<String>('pickFilePath');
      if (path == null) return null;
      if (path.startsWith('copy:')) {
        return PickedFiles([path.substring(5)], temporary: true);
      }
      return PickedFiles([path], temporary: false);
    }
    return pickFile();
  }

  /// One file to open, such as an `.atropos` package.
  Future<PickedFiles?> pickFile() async {
    final f = await openFile();
    if (f == null) return null;
    return PickedFiles([f.path], temporary: Platform.isAndroid);
  }

  /// A path to write a new file to. Android has no save dialog that yields a
  /// path, so the person picks a folder and the name is [suggestedName]
  /// (made unique).
  Future<String?> pickSaveFile(String suggestedName) async {
    if (Platform.isAndroid) {
      final dir = await pickFolder();
      if (dir == null) return null;
      return uniquePath(dir, suggestedName);
    }
    final loc = await getSaveLocation(suggestedName: suggestedName);
    return loc?.path;
  }

  /// `dir/name`, or `dir/name (2).ext` and so on when it exists.
  static String uniquePath(String dir, String name) {
    final sep = Platform.pathSeparator;
    var candidate = '$dir$sep$name';
    final dot = name.lastIndexOf('.');
    final stem = dot > 0 ? name.substring(0, dot) : name;
    final ext = dot > 0 ? name.substring(dot) : '';
    for (var i = 2; File(candidate).existsSync(); i++) {
      candidate = '$dir$sep$stem ($i)$ext';
    }
    return candidate;
  }
}

class PickedFiles {
  PickedFiles(this.paths, {required this.temporary});

  final List<String> paths;

  /// The picker copied these into the app's cache.
  final bool temporary;

  /// Removes the picker's temporary copies.
  Future<void> cleanUp() async {
    if (!temporary) return;
    for (final p in paths) {
      try {
        await File(p).delete();
      } catch (_) {
        // Already gone.
      }
    }
  }
}
