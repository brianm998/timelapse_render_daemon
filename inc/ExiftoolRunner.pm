package ExiftoolRunner;

use strict;

sub new {
    my ($class,
	$filename,
	$group,
	$exif_json_filename,
	)
	= @_;

  my $self =
    {
     filename => $filename,
     is_done => 0,
     is_running => 0,
     group => $group,
     exif_json_filename => $exif_json_filename,
     csv => Text::CSV->new ({ binary => 1, auto_diag => 1 }),
    };

  return bless $self, $class;
}

sub start($) {
  my ($self) = @_;

  my $fh;
  open $fh, "exiftool -csv $self->{filename} |" or die $!;
  $self->{fh} = \*$fh;
  $self->{is_running} = 1;
}

sub read($) {
 my ($self) = @_;

 unless(exists $self->{row1}) {
   $self->{row1} = $self->{csv}->getline($self->{fh});
   return;
 }
 unless(exists $self->{row2}) {
   $self->{row2} = $self->{csv}->getline($self->{fh});
   $self->finish();
   return;
 }
}

sub finish($) {
  my ($self) = @_;

  $self->{exif_data} = {};

  for(my $i = 0 ; $i < scalar @{$self->{row1}} ; $i++) {
    $self->{exif_data}{$self->{row1}[$i]} = $self->{row2}[$i];
  }

  close $self->{fh};
  $self->{is_done} = 1;

  if($self->group_is_done()) {
    my $total_exif_map = {};
    foreach my $runner (@{$self->{group}}) {
      $self->process_exif($total_exif_map, $self->{exif_data});
    }
    my $json_str = JSON->new->pretty->encode($total_exif_map);
    open my $filehandle, ">$self->{exif_json_filename}" or die "can't open $self->{exif_json_filename}: $!\n";
    print $filehandle $json_str;
    close $filehandle;
  }

  return $self->{exif_data};
}

# this adds tallies for a new exif map to the totals
sub process_exif() {
  my ($self, $total_exif_map, $new_exif_map) = @_;
  foreach my $key (keys %$new_exif_map) {
    next if($key eq 'Thumbnail Image');	# skip binary data
    next if($key eq 'Preview Image');
    next if($key eq 'Tiff Metering Image');
    my $new_value = $new_exif_map->{$key};
    if(exists $total_exif_map->{$key}{$new_value}) {
      $total_exif_map->{$key}{$new_value}++;
    } else {
      $total_exif_map->{$key}{$new_value} = 1;
    }
  }
}

sub group_is_done() {
  my ($self) = @_;

  foreach my $runner (@{$self->{group}}) {
    return 0 unless $runner->{is_done};
  }
  return $self->{is_done};
}
1;
