// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'maintenance.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$CheckEvent {

 Object get field0;



@override
bool operator ==(Object other) {
  final _this = this as CheckEvent;
  return identical(this, other) || (other.runtimeType == runtimeType&&other is CheckEvent&&const DeepCollectionEquality().equals(other.field0, _this.field0));
}


@override
int get hashCode {
  final _this = this as CheckEvent;
  return Object.hash(runtimeType,const DeepCollectionEquality().hash(_this.field0));
}

@override
String toString() {
  final _this = this as CheckEvent;
  return 'CheckEvent(field0: ${_this.field0})';
}


}

/// @nodoc
class $CheckEventCopyWith<$Res>  {
$CheckEventCopyWith(CheckEvent _, $Res Function(CheckEvent) __);
}


/// Adds pattern-matching-related methods to [CheckEvent].
extension CheckEventPatterns on CheckEvent {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( CheckEvent_Progress value)?  progress,TResult Function( CheckEvent_Done value)?  done,required TResult orElse(),}){
final _that = this;
switch (_that) {
case CheckEvent_Progress() when progress != null:
return progress(_that);case CheckEvent_Done() when done != null:
return done(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( CheckEvent_Progress value)  progress,required TResult Function( CheckEvent_Done value)  done,}){
final _that = this;
switch (_that) {
case CheckEvent_Progress():
return progress(_that);case CheckEvent_Done():
return done(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( CheckEvent_Progress value)?  progress,TResult? Function( CheckEvent_Done value)?  done,}){
final _that = this;
switch (_that) {
case CheckEvent_Progress() when progress != null:
return progress(_that);case CheckEvent_Done() when done != null:
return done(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( ProgressInfo field0)?  progress,TResult Function( CheckReportInfo field0)?  done,required TResult orElse(),}) {final _that = this;
switch (_that) {
case CheckEvent_Progress() when progress != null:
return progress(_that.field0);case CheckEvent_Done() when done != null:
return done(_that.field0);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( ProgressInfo field0)  progress,required TResult Function( CheckReportInfo field0)  done,}) {final _that = this;
switch (_that) {
case CheckEvent_Progress():
return progress(_that.field0);case CheckEvent_Done():
return done(_that.field0);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( ProgressInfo field0)?  progress,TResult? Function( CheckReportInfo field0)?  done,}) {final _that = this;
switch (_that) {
case CheckEvent_Progress() when progress != null:
return progress(_that.field0);case CheckEvent_Done() when done != null:
return done(_that.field0);case _:
  return null;

}
}

}

/// @nodoc


class CheckEvent_Progress extends CheckEvent {
  const CheckEvent_Progress(this.field0): super._();
  

@override final  ProgressInfo field0;

/// Create a copy of CheckEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CheckEvent_ProgressCopyWith<CheckEvent_Progress> get copyWith => _$CheckEvent_ProgressCopyWithImpl<CheckEvent_Progress>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CheckEvent_Progress&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode {
    return Object.hash(runtimeType,field0);
}

@override
String toString() {
    return 'CheckEvent.progress(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $CheckEvent_ProgressCopyWith<$Res> implements $CheckEventCopyWith<$Res> {
  factory $CheckEvent_ProgressCopyWith(CheckEvent_Progress value, $Res Function(CheckEvent_Progress) _then) = _$CheckEvent_ProgressCopyWithImpl;
@useResult
$Res call({
 ProgressInfo field0
});




}
/// @nodoc
class _$CheckEvent_ProgressCopyWithImpl<$Res>
    implements $CheckEvent_ProgressCopyWith<$Res> {
  _$CheckEvent_ProgressCopyWithImpl(this._self, this._then);

  final CheckEvent_Progress _self;
  final $Res Function(CheckEvent_Progress) _then;

/// Create a copy of CheckEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(CheckEvent_Progress(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as ProgressInfo,
  ));
}


}

/// @nodoc


class CheckEvent_Done extends CheckEvent {
  const CheckEvent_Done(this.field0): super._();
  

@override final  CheckReportInfo field0;

/// Create a copy of CheckEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CheckEvent_DoneCopyWith<CheckEvent_Done> get copyWith => _$CheckEvent_DoneCopyWithImpl<CheckEvent_Done>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CheckEvent_Done&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode {
    return Object.hash(runtimeType,field0);
}

@override
String toString() {
    return 'CheckEvent.done(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $CheckEvent_DoneCopyWith<$Res> implements $CheckEventCopyWith<$Res> {
  factory $CheckEvent_DoneCopyWith(CheckEvent_Done value, $Res Function(CheckEvent_Done) _then) = _$CheckEvent_DoneCopyWithImpl;
@useResult
$Res call({
 CheckReportInfo field0
});




}
/// @nodoc
class _$CheckEvent_DoneCopyWithImpl<$Res>
    implements $CheckEvent_DoneCopyWith<$Res> {
  _$CheckEvent_DoneCopyWithImpl(this._self, this._then);

  final CheckEvent_Done _self;
  final $Res Function(CheckEvent_Done) _then;

/// Create a copy of CheckEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(CheckEvent_Done(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as CheckReportInfo,
  ));
}


}

// dart format on
