// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'viewer.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$ImageEdit {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ImageEdit()';
}


}

/// @nodoc
class $ImageEditCopyWith<$Res>  {
$ImageEditCopyWith(ImageEdit _, $Res Function(ImageEdit) __);
}


/// Adds pattern-matching-related methods to [ImageEdit].
extension ImageEditPatterns on ImageEdit {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ImageEdit_RotateLeft value)?  rotateLeft,TResult Function( ImageEdit_RotateRight value)?  rotateRight,TResult Function( ImageEdit_FlipHorizontal value)?  flipHorizontal,TResult Function( ImageEdit_FlipVertical value)?  flipVertical,TResult Function( ImageEdit_Crop value)?  crop,TResult Function( ImageEdit_Resize value)?  resize,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ImageEdit_RotateLeft() when rotateLeft != null:
return rotateLeft(_that);case ImageEdit_RotateRight() when rotateRight != null:
return rotateRight(_that);case ImageEdit_FlipHorizontal() when flipHorizontal != null:
return flipHorizontal(_that);case ImageEdit_FlipVertical() when flipVertical != null:
return flipVertical(_that);case ImageEdit_Crop() when crop != null:
return crop(_that);case ImageEdit_Resize() when resize != null:
return resize(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ImageEdit_RotateLeft value)  rotateLeft,required TResult Function( ImageEdit_RotateRight value)  rotateRight,required TResult Function( ImageEdit_FlipHorizontal value)  flipHorizontal,required TResult Function( ImageEdit_FlipVertical value)  flipVertical,required TResult Function( ImageEdit_Crop value)  crop,required TResult Function( ImageEdit_Resize value)  resize,}){
final _that = this;
switch (_that) {
case ImageEdit_RotateLeft():
return rotateLeft(_that);case ImageEdit_RotateRight():
return rotateRight(_that);case ImageEdit_FlipHorizontal():
return flipHorizontal(_that);case ImageEdit_FlipVertical():
return flipVertical(_that);case ImageEdit_Crop():
return crop(_that);case ImageEdit_Resize():
return resize(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ImageEdit_RotateLeft value)?  rotateLeft,TResult? Function( ImageEdit_RotateRight value)?  rotateRight,TResult? Function( ImageEdit_FlipHorizontal value)?  flipHorizontal,TResult? Function( ImageEdit_FlipVertical value)?  flipVertical,TResult? Function( ImageEdit_Crop value)?  crop,TResult? Function( ImageEdit_Resize value)?  resize,}){
final _that = this;
switch (_that) {
case ImageEdit_RotateLeft() when rotateLeft != null:
return rotateLeft(_that);case ImageEdit_RotateRight() when rotateRight != null:
return rotateRight(_that);case ImageEdit_FlipHorizontal() when flipHorizontal != null:
return flipHorizontal(_that);case ImageEdit_FlipVertical() when flipVertical != null:
return flipVertical(_that);case ImageEdit_Crop() when crop != null:
return crop(_that);case ImageEdit_Resize() when resize != null:
return resize(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  rotateLeft,TResult Function()?  rotateRight,TResult Function()?  flipHorizontal,TResult Function()?  flipVertical,TResult Function( double left,  double top,  double right,  double bottom)?  crop,TResult Function( int maxSide)?  resize,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ImageEdit_RotateLeft() when rotateLeft != null:
return rotateLeft();case ImageEdit_RotateRight() when rotateRight != null:
return rotateRight();case ImageEdit_FlipHorizontal() when flipHorizontal != null:
return flipHorizontal();case ImageEdit_FlipVertical() when flipVertical != null:
return flipVertical();case ImageEdit_Crop() when crop != null:
return crop(_that.left,_that.top,_that.right,_that.bottom);case ImageEdit_Resize() when resize != null:
return resize(_that.maxSide);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  rotateLeft,required TResult Function()  rotateRight,required TResult Function()  flipHorizontal,required TResult Function()  flipVertical,required TResult Function( double left,  double top,  double right,  double bottom)  crop,required TResult Function( int maxSide)  resize,}) {final _that = this;
switch (_that) {
case ImageEdit_RotateLeft():
return rotateLeft();case ImageEdit_RotateRight():
return rotateRight();case ImageEdit_FlipHorizontal():
return flipHorizontal();case ImageEdit_FlipVertical():
return flipVertical();case ImageEdit_Crop():
return crop(_that.left,_that.top,_that.right,_that.bottom);case ImageEdit_Resize():
return resize(_that.maxSide);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  rotateLeft,TResult? Function()?  rotateRight,TResult? Function()?  flipHorizontal,TResult? Function()?  flipVertical,TResult? Function( double left,  double top,  double right,  double bottom)?  crop,TResult? Function( int maxSide)?  resize,}) {final _that = this;
switch (_that) {
case ImageEdit_RotateLeft() when rotateLeft != null:
return rotateLeft();case ImageEdit_RotateRight() when rotateRight != null:
return rotateRight();case ImageEdit_FlipHorizontal() when flipHorizontal != null:
return flipHorizontal();case ImageEdit_FlipVertical() when flipVertical != null:
return flipVertical();case ImageEdit_Crop() when crop != null:
return crop(_that.left,_that.top,_that.right,_that.bottom);case ImageEdit_Resize() when resize != null:
return resize(_that.maxSide);case _:
  return null;

}
}

}

/// @nodoc


class ImageEdit_RotateLeft extends ImageEdit {
  const ImageEdit_RotateLeft(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_RotateLeft);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ImageEdit.rotateLeft()';
}


}




/// @nodoc


class ImageEdit_RotateRight extends ImageEdit {
  const ImageEdit_RotateRight(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_RotateRight);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ImageEdit.rotateRight()';
}


}




/// @nodoc


class ImageEdit_FlipHorizontal extends ImageEdit {
  const ImageEdit_FlipHorizontal(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_FlipHorizontal);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ImageEdit.flipHorizontal()';
}


}




/// @nodoc


class ImageEdit_FlipVertical extends ImageEdit {
  const ImageEdit_FlipVertical(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_FlipVertical);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ImageEdit.flipVertical()';
}


}




/// @nodoc


class ImageEdit_Crop extends ImageEdit {
  const ImageEdit_Crop({required this.left, required this.top, required this.right, required this.bottom}): super._();
  

 final  double left;
 final  double top;
 final  double right;
 final  double bottom;

/// Create a copy of ImageEdit
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ImageEdit_CropCopyWith<ImageEdit_Crop> get copyWith => _$ImageEdit_CropCopyWithImpl<ImageEdit_Crop>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_Crop&&(identical(other.left, left) || other.left == left)&&(identical(other.top, top) || other.top == top)&&(identical(other.right, right) || other.right == right)&&(identical(other.bottom, bottom) || other.bottom == bottom));
}


@override
int get hashCode {
    return Object.hash(runtimeType,left,top,right,bottom);
}

@override
String toString() {
    return 'ImageEdit.crop(left: $left, top: $top, right: $right, bottom: $bottom)';
}


}

/// @nodoc
abstract mixin class $ImageEdit_CropCopyWith<$Res> implements $ImageEditCopyWith<$Res> {
  factory $ImageEdit_CropCopyWith(ImageEdit_Crop value, $Res Function(ImageEdit_Crop) _then) = _$ImageEdit_CropCopyWithImpl;
@useResult
$Res call({
 double left, double top, double right, double bottom
});




}
/// @nodoc
class _$ImageEdit_CropCopyWithImpl<$Res>
    implements $ImageEdit_CropCopyWith<$Res> {
  _$ImageEdit_CropCopyWithImpl(this._self, this._then);

  final ImageEdit_Crop _self;
  final $Res Function(ImageEdit_Crop) _then;

/// Create a copy of ImageEdit
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? left = null,Object? top = null,Object? right = null,Object? bottom = null,}) {
  return _then(ImageEdit_Crop(
left: null == left ? _self.left : left // ignore: cast_nullable_to_non_nullable
as double,top: null == top ? _self.top : top // ignore: cast_nullable_to_non_nullable
as double,right: null == right ? _self.right : right // ignore: cast_nullable_to_non_nullable
as double,bottom: null == bottom ? _self.bottom : bottom // ignore: cast_nullable_to_non_nullable
as double,
  ));
}


}

/// @nodoc


class ImageEdit_Resize extends ImageEdit {
  const ImageEdit_Resize({required this.maxSide}): super._();
  

 final  int maxSide;

/// Create a copy of ImageEdit
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ImageEdit_ResizeCopyWith<ImageEdit_Resize> get copyWith => _$ImageEdit_ResizeCopyWithImpl<ImageEdit_Resize>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ImageEdit_Resize&&(identical(other.maxSide, maxSide) || other.maxSide == maxSide));
}


@override
int get hashCode {
    return Object.hash(runtimeType,maxSide);
}

@override
String toString() {
    return 'ImageEdit.resize(maxSide: $maxSide)';
}


}

/// @nodoc
abstract mixin class $ImageEdit_ResizeCopyWith<$Res> implements $ImageEditCopyWith<$Res> {
  factory $ImageEdit_ResizeCopyWith(ImageEdit_Resize value, $Res Function(ImageEdit_Resize) _then) = _$ImageEdit_ResizeCopyWithImpl;
@useResult
$Res call({
 int maxSide
});




}
/// @nodoc
class _$ImageEdit_ResizeCopyWithImpl<$Res>
    implements $ImageEdit_ResizeCopyWith<$Res> {
  _$ImageEdit_ResizeCopyWithImpl(this._self, this._then);

  final ImageEdit_Resize _self;
  final $Res Function(ImageEdit_Resize) _then;

/// Create a copy of ImageEdit
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? maxSide = null,}) {
  return _then(ImageEdit_Resize(
maxSide: null == maxSide ? _self.maxSide : maxSide // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

// dart format on
